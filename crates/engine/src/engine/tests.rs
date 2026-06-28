use std::{
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionError, BranchKey, action::Action, context::CredentialContextExt,
    metadata::ActionMetadata, result::ActionResult, stateless::StatelessAction,
};
use nebula_core::{Dependencies, action_key, port_key, scope::Principal};
use nebula_storage_port::StorageError;
use nebula_storage_port::store::{ExecutionStore, NodeResultStore, WorkflowVersionStore};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, ErrorStrategy, NodeDefinition, Version, WorkflowConfig,
    WorkflowDefinition,
};

use super::*;
use crate::runtime::{
    ActionExecutor, DataPassingPolicy, InProcessRunner, registry::ActionRegistry,
};

// ── Variant A test fixtures ───────────────────────────────────────────
//
// Per Plan-agent R-NEW-7, fixtures register through
// [`ActionRegistry::register_stateless_instance`] (the
// test-only escape) so each test can vary key/version while the static
// `<X as Action>::metadata()` keeps a placeholder default.

struct EchoHandler;

impl Action for EchoHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.echo.static"), "Echo", "echoes input")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for EchoHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

struct FailHandler;

impl Action for FailHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.fail.static"), "Fail", "fails")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for FailHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Err(ActionError::fatal("intentional failure"))
    }
}

struct SlowHandler {
    delay: Duration,
}

impl Action for SlowHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.slow.static"), "Slow", "delays")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for SlowHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        tokio::select! {
            () = tokio::time::sleep(self.delay) => Ok(ActionResult::success(input)),
            () = ctx.cancellation().cancelled() => Err(ActionError::Cancelled),
        }
    }
}

// -- Helpers --

fn make_workflow(nodes: Vec<NodeDefinition>, connections: Vec<Connection>) -> WorkflowDefinition {
    let now = Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "test".into(),
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

fn make_workflow_with_config(
    nodes: Vec<NodeDefinition>,
    connections: Vec<Connection>,
    config: WorkflowConfig,
) -> WorkflowDefinition {
    let now = Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections,
        variables: HashMap::new(),
        config,
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

fn make_engine(registry: Arc<ActionRegistry>) -> (WorkflowEngine, MetricsRegistry) {
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
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

    let engine = WorkflowEngine::new(runtime, metrics.clone()).unwrap();
    (engine, metrics)
}

/// Test-only bundle of the spec-16 in-memory adapters.
///
/// Wires one `InMemoryExecutionStore` (plus the journal reader sharing
/// its core), node-result, checkpoint, idempotency, workflow, and
/// workflow-version adapters into the engine's [`ExecutionStores`] /
/// [`WorkflowStores`] bundles. It additionally exposes legacy-shaped
/// read/seed accessors (`get_state` / `load_node_output` /
/// `load_node_result` / `acquire_lease` / `is_idempotency_marked` /
/// `inject_state` / `inject_node_output` / `save_workflow`) so
/// post-execution assertions read the durable state the same way
/// they did against the old execution repo, mirroring the
/// production port path's scope/record semantics; test helpers call
/// [`crate::store_seam::single_tenant_scope`] for all store operations.
///
/// This is test scaffolding, not a production shim: the bundle's
/// fields are the same port traits production consumes; only the
/// legacy-shaped accessors are test-local conveniences.
#[derive(Clone)]
struct TestStores {
    execution: Arc<nebula_storage::InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<nebula_storage::InMemoryWorkflowVersionStore>,
}

impl TestStores {
    fn new() -> Self {
        let execution = Arc::new(nebula_storage::InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        // The workflow-row store shares the version store's map so
        // `save_with_published_version` commits the pair atomically
        // and the version-read path observes the same data.
        let versions = nebula_storage::InMemoryWorkflowVersionStore::new();
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

    /// The engine's execution-store bundle over these adapters.
    fn execution_stores(&self) -> crate::store_seam::ExecutionStores {
        crate::store_seam::ExecutionStores {
            execution: self.execution.clone(),
            journal: self.journal.clone(),
            node_results: self.node_results.clone(),
            checkpoints: self.checkpoints.clone(),
            idempotency: self.idempotency.clone(),
            resume_tokens: Arc::new(self.execution.resume_token_store()),
        }
    }

    /// The engine's workflow-store bundle over these adapters.
    fn workflow_stores(&self) -> crate::store_seam::WorkflowStores {
        crate::store_seam::WorkflowStores {
            workflow: self.workflow.clone(),
            versions: self.versions.clone(),
        }
    }

    /// Attach both bundles to `engine` (mirrors the production
    /// composition root, minus the tenancy decorator — every engine
    /// call uses the same placeholder scope so the raw adapters
    /// behave as one coherent tenant).
    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine
            .with_execution_stores(self.execution_stores())
            .with_workflow_stores(self.workflow_stores())
    }

    /// Persist a workflow definition as the published version 0 so the
    /// resume path's `get_published` lookup resolves it.
    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        let scope = crate::store_seam::single_tenant_scope();
        let definition = serde_json::to_value(wf).unwrap();
        self.versions
            .create(
                &scope,
                nebula_storage_port::dto::WorkflowVersionRecord {
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

    /// Legacy-shaped `(version, state)` read. Mirrors the production
    /// port path (`ExecutionStore::get`, scoped to the engine
    /// placeholder).
    async fn get_state(
        &self,
        id: ExecutionId,
    ) -> Result<Option<(u64, serde_json::Value)>, StorageError> {
        let scope = crate::store_seam::single_tenant_scope();
        Ok(self
            .execution
            .get(&scope, &id.to_string())
            .await?
            .map(|r| (r.version, r.state)))
    }

    /// Legacy-shaped single-node output read (the raw payload the
    /// engine persisted via `save_node_output`).
    async fn load_node_output(
        &self,
        id: ExecutionId,
        node: NodeKey,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        let scope = crate::store_seam::single_tenant_scope();
        Ok(self
            .node_results
            .load_node_output(&scope, &id.to_string(), node.as_str())
            .await?
            .map(|r| r.json))
    }

    /// Seed a crash-snapshot execution row directly (the port analog
    /// of the legacy `ExecutionRepo::create(id, wf, state_json)`
    /// state injection). The row is created at the port's baseline
    /// version with the given opaque state, exactly what the resume
    /// path reloads.
    async fn inject_state(
        &self,
        id: ExecutionId,
        workflow_id: WorkflowId,
        state: serde_json::Value,
    ) {
        let scope = crate::store_seam::single_tenant_scope();
        self.execution
            .create(&scope, &id.to_string(), &workflow_id.to_string(), state)
            .await
            .unwrap();
    }

    /// Seed a node's persisted output (the port analog of the legacy
    /// `ExecutionRepo::save_node_output`) so the resume path observes
    /// it as already produced.
    async fn inject_node_output(&self, id: ExecutionId, node: NodeKey, output: serde_json::Value) {
        let scope = crate::store_seam::single_tenant_scope();
        self.node_results
            .save_node_output(
                &scope,
                &id.to_string(),
                node.as_str(),
                crate::store_seam::node_output_record(output),
            )
            .await
            .unwrap();
    }

    /// Legacy-shaped typed-result read. Mirrors the production port
    /// path (`NodeResultStore::load_node_result`, scoped to the
    /// engine placeholder) — the port analog of the legacy
    /// `ExecutionRepo::load_node_result`. The returned record's
    /// `kind_tag`/`json` correspond to the legacy `kind`/`result`.
    async fn load_node_result(
        &self,
        id: ExecutionId,
        node: NodeKey,
    ) -> Result<Option<nebula_storage_port::dto::NodeResultRecord>, StorageError> {
        let scope = crate::store_seam::single_tenant_scope();
        self.node_results
            .load_node_result(&scope, &id.to_string(), node.as_str())
            .await
    }

    /// Legacy-shaped lease acquire. Mirrors the production port path
    /// (`ExecutionStore::acquire_lease`, scoped to the engine
    /// placeholder); returns `true` when a fencing token was
    /// granted — the port analog of the legacy
    /// `ExecutionRepo::acquire_lease` `bool`.
    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: std::time::Duration,
    ) -> Result<bool, StorageError> {
        let scope = crate::store_seam::single_tenant_scope();
        Ok(self
            .execution
            .acquire_lease(&scope, &id.to_string(), holder, ttl)
            .await?
            .is_some())
    }

    /// Non-mutating dedup-state read. Mirrors the production path's
    /// idempotency mark (scope + `{execution_id}:{node}:{attempt}`)
    /// without perturbing it — the port analog of the
    /// legacy `ExecutionRepo::check_idempotency`.
    fn is_idempotency_marked(&self, id: ExecutionId, node: NodeKey, attempt: u32) -> bool {
        let scope = crate::store_seam::single_tenant_scope();
        self.idempotency
            .is_marked(&scope, &id.to_string(), node.as_str(), attempt)
    }
}

// -- Tests --

#[tokio::test]
async fn single_node_workflow() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "echo", "core", "echo").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hello"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&n), Some(&serde_json::json!("hello")));
}

#[tokio::test]
async fn linear_two_node_workflow() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(n2.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(n1.clone(), n2.clone())],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(42),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&n1), Some(&serde_json::json!(42)));
    // B echoes its input, which is A's output (42)
    assert_eq!(result.node_output(&n2), Some(&serde_json::json!(42)));
}

#[tokio::test]
async fn diamond_workflow() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a.clone(), c.clone()),
            Connection::new(b.clone(), d.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("start"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_outputs.len(), 4);
    assert_eq!(result.node_output(&a), Some(&serde_json::json!("start")));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!("start")));
    assert_eq!(result.node_output(&c), Some(&serde_json::json!("start")));
    // Join node gets merged outputs from b and c
    let d_output = result.node_output(&d).unwrap();
    assert!(d_output.is_object());
}

#[tokio::test]
async fn failing_node_stops_execution() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, _) = make_engine(registry);

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let n3 = node_key!("n3");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(n2.clone(), "B", "core", "fail").unwrap(),
            NodeDefinition::new(n3.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(n1.clone(), n2.clone()),
            Connection::new(n2.clone(), n3.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure());
    assert!(result.node_output(&n1).is_some());
    assert!(result.node_output(&n2).is_none());
    assert!(result.node_output(&n3).is_none());
}

#[tokio::test]
async fn missing_action_key_returns_error() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "A", "core", "unknown").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok even when a node fails");

    // When action key is not in registry, the node fails and execution result is failure
    assert!(!result.is_success());
}

#[tokio::test]
async fn empty_workflow_returns_planning_error() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);

    let wf = make_workflow(vec![], vec![]);
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;

    assert!(matches!(result, Err(EngineError::PlanningFailed(_))));
}

#[tokio::test]
async fn telemetry_events_emitted() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, metrics) = make_engine(registry);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "echo", "core", "echo").unwrap()],
        vec![],
    );

    engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("test"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .unwrap()
            .get()
            > 0
    );
    assert!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
            .unwrap()
            .get()
            > 0
    );
}

#[tokio::test]
async fn metrics_recorded_on_failure() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, metrics) = make_engine(registry);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "fail", "core", "fail").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure());
    assert!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
            .unwrap()
            .get()
            > 0
    );
    assert!(
        metrics
            .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
            .unwrap()
            .get()
            > 0
    );
}

#[tokio::test]
async fn trigger_context_construction_is_usable_in_engine() {
    let base = Arc::new(
        nebula_core::BaseContext::builder(nebula_core::scope::Scope::default())
            .principal(Principal::System)
            .cancellation(CancellationToken::new())
            .build()
            .expect("scope + principal must produce a valid BaseContext"),
    );
    let ctx = nebula_action::TriggerRuntimeContext::new(base, WorkflowId::new(), node_key!("test"));
    assert!(!ctx.has_credential_id("missing").await);
    assert!(
        ctx.schedule_after(std::time::Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(
        ctx.emit_execution(serde_json::json!({"event":"tick"}), None)
            .await
            .is_err()
    );
}

// -- Frontier-specific test handlers --

struct SkipHandler;

impl Action for SkipHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.skip.static"), "Skip", "skips")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for SkipHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::skip("skipped by test"))
    }
}

struct BranchHandler {
    selected: BranchKey,
}

impl Action for BranchHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.branch.static"), "Branch", "branches")
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for BranchHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Branch {
            selected: self.selected.clone(),
            output: nebula_action::output::ActionOutput::Value(input),
            alternatives: HashMap::new(),
        })
    }
}

// -- Frontier-specific tests --

/// A → Branch(selects "true") → B (branch_key="true") / C (branch_key="false") → D
/// Only B should execute; C should be skipped; D should still run (via B).
#[tokio::test]
async fn branch_workflow_only_selected_path_executes() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("branch"), "Branch", "branches"),
        BranchHandler {
            selected: nebula_action::branch_key!("true"),
        },
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "branch").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()).with_from_port(port_key!("true")),
            Connection::new(a.clone(), c.clone()).with_from_port(port_key!("false")),
            Connection::new(b.clone(), d.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    // A executed (branch node)
    assert!(result.node_output(&a).is_some());
    // B executed (true branch)
    assert!(result.node_output(&b).is_some());
    // C was NOT executed (false branch, skipped)
    assert!(result.node_output(&c).is_none());
    // D executed (received input from B only)
    assert!(result.node_output(&d).is_some());
}

/// A → B(skip) → C. Verify C is skipped and doesn't execute.
#[tokio::test]
async fn skip_propagation() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("skip"), "Skip", "always skips"),
        SkipHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "skip").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    // Execution succeeds overall (skip is not a failure)
    assert!(result.is_success());
    // A executed
    assert!(result.node_output(&a).is_some());
    // B executed but produced Skip result (no output stored since skip has no output)
    assert!(result.node_output(&b).is_none());
    // C was skipped (never executed)
    assert!(result.node_output(&c).is_none());
}

/// A → B(fails) --OnError--> C. Verify C receives error data and execution succeeds.
#[tokio::test]
async fn error_routing_with_handler() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "fail").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b.clone(), c.clone()).with_from_port(port_key!("error")),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    // Execution succeeds because the error was handled
    assert!(result.is_success());
    // A executed
    assert!(result.node_output(&a).is_some());
    // B failed but error data was stored
    assert!(result.node_output(&b).is_some());
    // C executed with error data from B
    let c_output = result.node_output(&c).unwrap();
    assert!(c_output.get("error").is_some());
}

/// A → B(fails) → C (Always). No OnError handler → fail-fast (same as today).
#[tokio::test]
async fn error_without_handler_fails_fast() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "fail").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(b, c.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("input"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure());
    assert!(result.node_output(&a).is_some());
    // B failed, no error handler → fail-fast
    assert!(result.node_output(&c).is_none());
}

/// A → B with OnResult(Success) condition. B should run when A succeeds.
#[tokio::test]
async fn conditional_edge_on_result() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone())],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hello"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_output(&a), Some(&serde_json::json!("hello")));
    assert_eq!(result.node_output(&b), Some(&serde_json::json!("hello")));
}

/// Diamond with mixed conditions:
/// A → B (Always), A → C (OnResult{Success}), B → D, C → D
/// All should execute when A succeeds.
#[tokio::test]
async fn diamond_with_mixed_conditions() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let d = node_key!("d");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
            NodeDefinition::new(d.clone(), "D", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()), // Always
            Connection::new(a.clone(), c.clone()),
            Connection::new(b.clone(), d.clone()),
            Connection::new(c.clone(), d.clone()),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("start"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.node_outputs.len(), 4);
    assert!(result.node_output(&a).is_some());
    assert!(result.node_output(&b).is_some());
    assert!(result.node_output(&c).is_some());
    // D should have merged input from B and C
    let d_output = result.node_output(&d).unwrap();
    assert!(d_output.is_object());
}

// -- ExecutionRepo persistence tests --

#[tokio::test]
async fn persists_execution_state_on_success() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "echo", "core", "echo").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hello"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());

    // Verify state was persisted
    let entry = stores.get_state(result.execution_id).await.unwrap();
    assert!(entry.is_some(), "execution state should be persisted");
    let (version, state) = entry.unwrap();
    assert!(version >= 2, "repo version should have been bumped");
    assert_eq!(
        state.get("status").and_then(|s| s.as_str()),
        Some("completed")
    );

    // Verify node output was saved
    let node_output = stores
        .load_node_output(result.execution_id, n)
        .await
        .unwrap();
    assert_eq!(node_output, Some(serde_json::json!("hello")));
}

#[tokio::test]
async fn persists_execution_state_on_failure() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "fail", "core", "fail").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure());

    // Verify final state was persisted as failed
    let entry = stores.get_state(result.execution_id).await.unwrap();
    assert!(entry.is_some(), "execution state should be persisted");
    let (_version, state) = entry.unwrap();
    assert_eq!(state.get("status").and_then(|s| s.as_str()), Some("failed"));
}

#[tokio::test]
async fn persists_node_outputs_for_multi_node_workflow() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(n2.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(n1.clone(), n2.clone())],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(42),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());

    // Both node outputs should be persisted (the port NodeResultStore
    // is ISP-segregated with no bulk-output enumerator, so each node
    // is checked individually — the workflow has exactly these two).
    let out1 = stores
        .load_node_output(result.execution_id, n1)
        .await
        .unwrap();
    let out2 = stores
        .load_node_output(result.execution_id, n2)
        .await
        .unwrap();
    assert_eq!(out1, Some(serde_json::json!(42)));
    assert_eq!(out2, Some(serde_json::json!(42)));
}

// -- Budget enforcement tests --

#[tokio::test]
async fn budget_max_duration_exceeded() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("slow"), "Slow", "sleeps"),
        SlowHandler {
            delay: Duration::from_millis(100),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    // Slow → Echo. Budget allows only 1ms.
    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "Slow", "core", "slow").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a, b)],
    );

    let budget = ExecutionBudget::default().with_max_duration(Duration::from_millis(1));

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("data"),
            budget,
        )
        .await
        .unwrap();

    // The slow action takes >1ms, so budget should trigger before
    // the next node is dispatched.
    assert!(result.is_failure());
}

#[tokio::test]
async fn budget_max_output_bytes_exceeded() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let (engine, _) = make_engine(registry);

    // A → B. Each echoes a payload. Budget allows very few bytes.
    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a, b)],
    );

    // Budget: max 5 bytes of total output (the JSON "hello" is 7 bytes)
    let budget = ExecutionBudget::default().with_max_output_bytes(5);

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hello"),
            budget,
        )
        .await
        .unwrap();

    // A's output exceeds 5 bytes → budget violation before B runs
    assert!(result.is_failure());
}

// -- Error strategy tests --

#[tokio::test]
async fn error_strategy_continue_on_error_skips_dependents() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, _) = make_engine(registry);

    // Entry → [Fail, Echo(C)]
    // Fail → B
    // With ContinueOnError: Fail fails, B is skipped, C still runs.
    let entry = node_key!("entry");
    let fail_node = node_key!("fail_node");
    let b = node_key!("b");
    let c = node_key!("c");

    let config = WorkflowConfig {
        error_strategy: ErrorStrategy::ContinueOnError,
        ..WorkflowConfig::default()
    };

    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(entry.clone(), "Entry", "core", "echo").unwrap(),
            NodeDefinition::new(fail_node.clone(), "Fail", "core", "fail").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(entry.clone(), fail_node.clone()),
            Connection::new(entry.clone(), c.clone()),
            Connection::new(fail_node, b.clone()),
        ],
        config,
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("data"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    // Workflow completes (not fail-fast)
    assert!(result.is_success() || result.status == ExecutionStatus::Completed);
    // Entry ran
    assert!(result.node_output(&entry).is_some());
    // C is independent and should have run
    assert!(result.node_output(&c).is_some());
    // B depends on the failed node — should be skipped (no output)
    assert!(result.node_output(&b).is_none());
}

#[tokio::test]
async fn error_strategy_ignore_errors_continues_downstream() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "always fails"),
        FailHandler,
    );

    let (engine, _) = make_engine(registry);

    // A(fail) → B(echo)
    // With IgnoreErrors: A fails but B should still run with null input
    let a = node_key!("a");
    let b = node_key!("b");

    let config = WorkflowConfig {
        error_strategy: ErrorStrategy::IgnoreErrors,
        ..WorkflowConfig::default()
    };

    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "fail").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone())],
        config,
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("data"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    // Workflow should complete successfully
    assert_eq!(result.status, ExecutionStatus::Completed);
    // A's output was replaced with null
    assert_eq!(result.node_output(&a), Some(&serde_json::json!(null)));
    // B ran and received null as input
    assert!(result.node_output(&b).is_some());
    assert_eq!(result.node_output(&b), Some(&serde_json::json!(null)));
}

// -- resume_execution tests --

#[tokio::test]
async fn resume_requires_execution_repo() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);
    // No execution / workflow store bundles attached.
    let err = engine
        .resume_execution(
            &crate::store_seam::single_tenant_scope(),
            ExecutionId::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("execution_repo")),
        "expected no-execution_repo error, got: {err}"
    );
}

#[tokio::test]
async fn resume_requires_workflow_repo() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);
    let stores = TestStores::new();
    let engine = engine.with_execution_stores(stores.execution_stores());
    // No workflow store attached.
    let err = engine
        .resume_execution(
            &crate::store_seam::single_tenant_scope(),
            ExecutionId::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("workflow_repo")),
        "expected no-workflow_repo error, got: {err}"
    );
}

#[tokio::test]
async fn resume_returns_error_for_missing_execution() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);
    let stores = TestStores::new();
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "echo", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;
    let engine = stores.attach(engine);

    let err = engine
        .resume_execution(
            &crate::store_seam::single_tenant_scope(),
            ExecutionId::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("not found")),
        "expected not-found error, got: {err}"
    );
}

#[tokio::test]
async fn resume_returns_error_for_terminal_execution() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n, "echo", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;
    let engine = stores.attach(engine);

    // Run to completion first.
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hi"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(result.is_success());

    // Now resume the completed execution — should fail.
    let err = engine
        .resume_execution(
            &crate::store_seam::single_tenant_scope(),
            result.execution_id,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::PlanningFailed(ref msg) if msg.contains("terminal")),
        "expected terminal-state error, got: {err}"
    );
}

#[tokio::test]
async fn resume_executes_remaining_nodes_after_crash() {
    // Simulate a 3-node linear workflow (n1 → n2 → n3) where n1 completed
    // before the crash. We manually inject the partially completed state into
    // the repos and verify that resume runs n2 and n3.
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let n3 = node_key!("n3");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(n2.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(n3.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(n1.clone(), n2.clone()),
            Connection::new(n2.clone(), n3.clone()),
        ],
    );
    stores.save_workflow(&wf).await;

    // Manually build a partial execution state where n1 is Completed but
    // n2 and n3 are still Pending (simulating a crash after n1 finished).
    let execution_id = ExecutionId::new();
    let node_ids = vec![n1.clone(), n2.clone(), n3.clone()];
    let mut exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
    exec_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    // Mark n1 as completed.
    exec_state
        .node_states
        .get_mut(&n1)
        .unwrap()
        .transition_to(NodeState::Ready)
        .unwrap();
    exec_state
        .node_states
        .get_mut(&n1)
        .unwrap()
        .transition_to(NodeState::Running)
        .unwrap();
    exec_state
        .node_states
        .get_mut(&n1)
        .unwrap()
        .transition_to(NodeState::Completed)
        .unwrap();

    let state_json = serde_json::to_value(&exec_state).unwrap();
    stores.inject_state(execution_id, wf.id, state_json).await;

    // Persist n1's output.
    stores
        .inject_node_output(execution_id, n1.clone(), serde_json::json!("from_n1"))
        .await;

    let engine = stores.attach(engine);

    let scope = crate::store_seam::single_tenant_scope();
    let result = engine.resume_execution(&scope, execution_id).await.unwrap();

    assert!(result.is_success(), "resume should complete successfully");
    assert_eq!(result.execution_id, execution_id);
    // n1's output comes from the persisted outputs
    assert_eq!(result.node_output(&n1), Some(&serde_json::json!("from_n1")));
    // n2 and n3 should have been executed and produced outputs
    assert!(
        result.node_output(&n2).is_some(),
        "n2 should have been re-executed"
    );
    assert!(
        result.node_output(&n3).is_some(),
        "n3 should have been re-executed"
    );
}

/// Regression for [#321](https://github.com/vanyastaff/nebula/issues/321).
///
/// The setup-failure branch of `run_frontier` (parameter resolution
/// error before the action is spawned) routed the failure through
/// `handle_node_failure` but SKIPPED the `checkpoint_node` call the
/// runtime-failure branch makes. A crash between setup-failure
/// handling and the next final-state checkpoint therefore lost both
/// the node's `Failed` state and any OnError / ContinueOnError
/// edge-routing already applied in memory by `handle_node_failure`.
// (durability precedes visibility, /
/// #297).
///
/// This test covers the fix in two parts:
///   1. Running a ContinueOnError workflow with one node that fails at parameter resolution.
///      Symmetric persistence means the frontier loop emits one extra `transition()` against
///      the repo — observable as an additional repo-version bump (create → setup-failure
///      checkpoint → final = v3 vs the pre-fix create → final = v2).
///   2. Simulating a crash at that intermediate checkpoint by injecting a matching state
///      snapshot into a fresh repo and resuming. The resumed engine must keep the node in
///      `Failed` (terminal states are not reset by `resume_execution`) and must NOT re-execute
///      the node from scratch.
#[tokio::test]
async fn setup_failure_persists_before_final_checkpoint() {
    use nebula_workflow::ParamValue;

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    // `ContinueOnError` ensures `handle_node_failure` returns `None`
    // so the frontier loop reaches the new setup-failure checkpoint.
    // FailFast would return early (cancel + propagate) before the
    // branch this test is exercising; the same durability gap exists
    // there, but this is the exercise path that lets the test
    // observe the new transition directly.
    let b = node_key!("b");
    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(b.clone(), "B", "core", "echo")
                .unwrap()
                .with_parameter("bad", ParamValue::template("Hello {{ unclosed")),
        ],
        vec![],
        WorkflowConfig {
            error_strategy: ErrorStrategy::ContinueOnError,
            ..WorkflowConfig::default()
        },
    );
    // Part 1: run the workflow and observe the extra checkpoint via
    // the repo-version counter.
    let stores1 = TestStores::new();
    stores1.save_workflow(&wf).await;
    let (engine1, _) = make_engine(registry.clone());
    let engine1 = stores1.attach(engine1);

    let result = engine1
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    let (version, final_state) = stores1
        .get_state(result.execution_id)
        .await
        .unwrap()
        .expect("execution state must be persisted");
    // Using `>=` rather than `==` so a future legitimate mid-execution
    // checkpoint (e.g. a per-status-transition persist) does not break
    // this test. The regression signal is preserved either way: the
    // pre-fix path lands one commit short (create + final only). The
    // port seeds create at v=0 (the legacy ExecutionRepo seeded v=1,
    // so the legacy threshold was >=3); the setup-failure checkpoint
    // + final are two commits past create.
    assert!(
        version >= 2,
        "expected at least two commits past the port's v=0 create: \
             setup-failure checkpoint (the fix) + final. Pre-fix path \
             skips the setup-failure checkpoint and lands at v1; got \
             {version}"
    );
    assert_eq!(
        final_state
            .get("node_states")
            .and_then(|ns| ns.get(b.as_str()))
            .and_then(|nb| nb.get("state"))
            .and_then(|v| v.as_str()),
        Some("failed"),
        "final persisted state must record node B as Failed"
    );
    assert!(
        result.node_errors.contains_key(&b),
        "execution result must carry the setup-failure error for B"
    );

    // Part 2: simulate a crash at the intermediate checkpoint. Build
    // a state snapshot matching what the setup-failure checkpoint
    // writes (status=Running, node B Failed with error message) and
    // resume in a fresh repo.
    let execution_id = ExecutionId::new();
    let node_ids = vec![b.clone()];
    let mut crashed_state = ExecutionState::new(execution_id, wf.id, &node_ids);
    crashed_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    // Mirror spawn_node's override on parameter-resolution failure:
    // the node was still Pending when resolution failed, so we use
    // override_node_state (Pending → Failed is not a valid forward
    // transition). The bump is implicit.
    crashed_state
        .override_node_state(b.clone(), NodeState::Failed)
        .unwrap();
    if let Some(ns) = crashed_state.node_states.get_mut(&b) {
        ns.error_message = Some("parameter resolution failed: template parse error".into());
    }

    let stores2 = TestStores::new();
    stores2.save_workflow(&wf).await;
    stores2
        .inject_state(
            execution_id,
            wf.id,
            serde_json::to_value(&crashed_state).unwrap(),
        )
        .await;

    let (engine2, _) = make_engine(registry);
    let engine2 = stores2.attach(engine2);
    let scope = crate::store_seam::single_tenant_scope();
    let resumed = engine2
        .resume_execution(&scope, execution_id)
        .await
        .unwrap();

    // Resume must land in a terminal status — the Failed node is
    // already terminal, so the frontier has nothing to run.
    assert!(
        resumed.status.is_terminal(),
        "resume must reach a terminal status, got {:?}",
        resumed.status
    );

    // Node B must still carry its setup-failure error: resume leaves
    // terminal nodes untouched (engine.rs §resume_execution step 7).
    // If B had been re-dispatched, its attempts vector would grow or
    // the error message would be overwritten by a new failure.
    let persisted = stores2
        .get_state(execution_id)
        .await
        .unwrap()
        .expect("state must still be persisted after resume");
    assert_eq!(
        persisted
            .1
            .get("node_states")
            .and_then(|ns| ns.get(b.as_str()))
            .and_then(|nb| nb.get("state"))
            .and_then(|v| v.as_str()),
        Some("failed"),
        "resume must not have reset node B's terminal Failed state"
    );
    assert!(
        resumed
            .node_errors
            .get(&b)
            .is_some_and(|err| err.contains("parameter resolution failed")),
        "resumed node B must still report the injected setup-failure \
             message; re-execution would have replaced it. errors: {:?}",
        resumed.node_errors
    );
}

// -- Crash-window regression tests for #297 / D2 --

/// Wraps an inner [`ExecutionStore`] and returns `Err` on the Nth
/// `commit()` call (1-indexed). All other trait methods delegate.
/// Used to simulate a storage failure during `checkpoint_node`'s
/// fencing-gated transition commit (the port analog of the legacy
/// `transition()` failure point).
#[derive(Debug)]
struct FailAtCommitN {
    inner: Arc<nebula_storage::InMemoryExecutionStore>,
    fail_on: u32,
    calls: AtomicU32,
}

impl FailAtCommitN {
    fn new(inner: Arc<nebula_storage::InMemoryExecutionStore>, fail_on: u32) -> Self {
        Self {
            inner,
            fail_on,
            calls: AtomicU32::new(0),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionStore for FailAtCommitN {
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

    async fn get(
        &self,
        scope: &Scope,
        id: &str,
    ) -> Result<Option<nebula_storage_port::dto::ExecutionRecord>, StorageError> {
        self.inner.get(scope, id).await
    }

    async fn commit(
        &self,
        batch: nebula_storage_port::TransitionBatch,
    ) -> Result<nebula_storage_port::TransitionOutcome, StorageError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.fail_on {
            return Err(StorageError::Connection(format!(
                "injected commit failure at call #{n}"
            )));
        }
        self.inner.commit(batch).await
    }

    async fn acquire_lease(
        &self,
        scope: &Scope,
        id: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<nebula_storage_port::FencingToken>, StorageError> {
        self.inner.acquire_lease(scope, id, holder, ttl).await
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: nebula_storage_port::FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        self.inner.renew_lease(scope, id, token, ttl).await
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: nebula_storage_port::FencingToken,
    ) -> Result<bool, StorageError> {
        self.inner.release_lease(scope, id, token).await
    }

    async fn list_all_running(
        &self,
    ) -> Result<Vec<nebula_storage_port::dto::ExecutionRecord>, StorageError> {
        self.inner.list_all_running().await
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

/// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2).
///
/// When `checkpoint_node` fails on the runtime-failure branch, the
/// engine MUST abort the node's progression: the `Failed` state is
/// not durably persisted, therefore no OnError successor may be
/// spawned and no `NodeFailed` event may be emitted. Pre-fix the
/// checkpoint error was silently logged (`tracing::warn!`) and
/// `handle_node_failure` had already routed the OnError edge in
/// memory, so the successor `B` was spawned off an undurable
/// failure decision — the "durability precedes visibility"
/// invariant was violated.
#[tokio::test]
async fn runtime_failure_checkpoint_error_aborts_before_edge_routing() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        FailHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        EchoHandler,
    );

    // A (fail) --OnError--> B (echo). ContinueOnError so the frontier
    // loop reaches the failure branch (FailFast would early-return
    // before checkpoint).
    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "fail").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone()).with_from_port(port_key!("error"))],
        WorkflowConfig {
            error_strategy: ErrorStrategy::ContinueOnError,
            ..WorkflowConfig::default()
        },
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    // First commit() call corresponds to the checkpoint_node
    // invocation after A's runtime failure (`create` is not a
    // commit). Fail it.
    let base = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let failing = Arc::new(FailAtCommitN::new(base, 1));
    let execution_stores = crate::store_seam::ExecutionStores {
        execution: failing,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(nebula_storage::InMemoryResumeTokenStore::standalone()),
    };

    let (engine, _) = make_engine(registry);
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut event_rx = event_bus.subscribe();
    let engine = engine
        .with_execution_stores(execution_stores)
        .with_workflow_stores(stores.workflow_stores())
        .with_event_bus(event_bus);

    let _ = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;

    // Drop engine so the event channel closes; drain.
    drop(engine);
    let mut events = Vec::new();
    while let Some(e) = event_rx.recv().await {
        events.push(e);
    }

    let b_started = events.iter().any(|e| {
        matches!(
            e,
            ExecutionEvent::NodeStarted { node_key, .. } if node_key == &b
        )
    });
    assert!(
        !b_started,
        "B must not be spawned after A's checkpoint failed — \
             checkpoint must precede edge routing (§11.5, #297). events: {events:#?}"
    );

    let a_failed_announced = events.iter().any(|e| {
        matches!(
            e,
            ExecutionEvent::NodeFailed { node_key, .. } if node_key == &a
        )
    });
    assert!(
        !a_failed_announced,
        "NodeFailed must not fire when A's checkpoint failed — \
             external observers must never see a transition the store \
             did not commit (§11.5, #297). events: {events:#?}"
    );
}

/// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2).
///
/// `IgnoreErrors` strategy recovers a failed node to `Completed`. The
/// recovery MUST survive a checkpoint boundary: the sequence
/// `Failed → Completed` in memory must be persisted as `Completed`
/// before successors (which see a "success with null" payload) are
/// routed. Pre-fix, `handle_node_failure` applied the override in
/// memory and routed edges, then the outer `if state == Failed`
/// guard skipped the checkpoint — so persistence lagged the
/// observable recovery by up to one final-state flush.
#[tokio::test]
async fn ignore_errors_persists_recovered_completed_state() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        FailHandler,
    );

    let a = node_key!("a");
    let wf = make_workflow_with_config(
        vec![NodeDefinition::new(a.clone(), "A", "core", "fail").unwrap()],
        vec![],
        WorkflowConfig {
            error_strategy: ErrorStrategy::IgnoreErrors,
            ..WorkflowConfig::default()
        },
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "IgnoreErrors workflow must finish Completed, got {:?}",
        result.status
    );

    let (version, final_state) = stores
        .get_state(result.execution_id)
        .await
        .unwrap()
        .expect("state must be persisted");

    // Expected commits past the port's v=0 create baseline:
    // IgnoreErrors recovery checkpoint (v1) → final (v2). Pre-fix
    // path skips the recovery checkpoint and lands at v1. (The
    // legacy ExecutionRepo seeded create at v=1, so the legacy
    // threshold was >=3; the port seeds at v=0.) Using `>=` so
    // later legitimate checkpoint additions do not break the signal.
    assert!(
        version >= 2,
        "expected at least two commits past create: recovery \
             checkpoint + final. Pre-fix path persists the recovered \
             state only at the final flush; got {version}"
    );

    assert_eq!(
        final_state
            .get("node_states")
            .and_then(|ns| ns.get(a.as_str()))
            .and_then(|na| na.get("state"))
            .and_then(|v| v.as_str()),
        Some("completed"),
        "IgnoreErrors must persist the recovered Completed state, \
             not the intermediate Failed"
    );
}

/// Regression for [#297](https://github.com/vanyastaff/nebula/issues/297) (D2) —
/// setup-failure branch symmetry with runtime-failure branch.
///
/// Parameter-resolution failure goes through the setup-failure arm
/// of `run_frontier`. The checkpoint-before-routing discipline
/// must hold there too: if the setup-failure checkpoint errors,
/// the engine aborts instead of logging-and-continuing onto the
/// OnError successor.
#[tokio::test]
async fn setup_failure_checkpoint_error_aborts_before_edge_routing() {
    use nebula_workflow::ParamValue;
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        EchoHandler,
    );

    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo")
                .unwrap()
                .with_parameter("bad", ParamValue::template("Hello {{ unclosed")),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone()).with_from_port(port_key!("error"))],
        WorkflowConfig {
            error_strategy: ErrorStrategy::ContinueOnError,
            ..WorkflowConfig::default()
        },
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    let base = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let failing = Arc::new(FailAtCommitN::new(base, 1));
    let execution_stores = crate::store_seam::ExecutionStores {
        execution: failing,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(nebula_storage::InMemoryResumeTokenStore::standalone()),
    };

    let (engine, _) = make_engine(registry);
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(64);
    let mut event_rx = event_bus.subscribe();
    let engine = engine
        .with_execution_stores(execution_stores)
        .with_workflow_stores(stores.workflow_stores())
        .with_event_bus(event_bus);

    let _ = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;

    drop(engine);
    let mut events = Vec::new();
    while let Some(e) = event_rx.recv().await {
        events.push(e);
    }

    let b_started = events.iter().any(|e| {
        matches!(
            e,
            ExecutionEvent::NodeStarted { node_key, .. } if node_key == &b
        )
    });
    assert!(
        !b_started,
        "B must not be spawned after A's setup-failure checkpoint \
             failed (§11.5, #297). events: {events:#?}"
    );
}

/// Regression for PR [#436](https://github.com/vanyastaff/nebula/pull/436)
/// review (Copilot) — the OnError input payload
/// (`{error, node_id}`) must be staged into `outputs[failed_node]`
/// BEFORE `checkpoint_node` commits the failure, so that a crashed-
/// then-resumed workflow loads it via `load_all_outputs` rather
/// than finding the OnError successor's input missing.
#[tokio::test]
async fn on_error_payload_is_persisted_before_checkpoint_commits() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("fail"), "Fail", "fails"),
        FailHandler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        EchoHandler,
    );

    let a = node_key!("a");
    let b = node_key!("b");
    let wf = make_workflow_with_config(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "fail").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(a.clone(), b.clone()).with_from_port(port_key!("error"))],
        WorkflowConfig {
            error_strategy: ErrorStrategy::ContinueOnError,
            ..WorkflowConfig::default()
        },
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    // The OnError handler B completed, so the workflow reports
    // success (ContinueOnError + handled error).
    assert!(result.is_success(), "status: {:?}", result.status);

    // The OnError input payload must have been loadable from the
    // persistence store — i.e. captured by the checkpoint that
    // commits A's Failed state, not written ephemerally after.
    let persisted = stores
        .load_node_output(result.execution_id, a.clone())
        .await
        .unwrap()
        .expect(
            "outputs[A] must be persisted: resume's load_all_outputs \
                 depends on it for the OnError handler's input",
        );

    let error_field = persisted.get("error").and_then(|v| v.as_str());
    let node_id_field = persisted.get("node_id").and_then(|v| v.as_str());
    assert_eq!(
        node_id_field,
        Some(a.as_str()),
        "persisted payload must carry node_id for the OnError \
             handler; got {persisted:?}"
    );
    assert!(
        error_field.is_some_and(|s| s.contains("intentional failure")),
        "persisted payload must carry the error message; got {persisted:?}"
    );
}

// -- Durable idempotency tests --

/// Pre-marking a node's idempotency key causes the engine to skip execution
/// and load the persisted output instead of re-running the action.
#[tokio::test]
async fn idempotency_check_prevents_double_execution() {
    use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

    // Track how many times the handler is actually invoked.
    let call_count = Arc::new(AtomicU32::new(0));
    let call_count_clone = call_count.clone();

    struct CountingHandler {
        count: Arc<AtomicU32>,
    }

    impl Action for CountingHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("counting.static"), "Counting", "counts calls")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for CountingHandler {
        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            self.count.fetch_add(1, AOrdering::Relaxed);
            Ok(ActionResult::success(input))
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("counting"), "Counting", "counts calls"),
        CountingHandler {
            count: call_count_clone,
        },
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = engine.with_execution_stores(stores.execution_stores());

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "count_node", "core", "counting").unwrap()],
        vec![],
    );

    // Run the workflow once — node should execute and its idempotency key
    // should be recorded.
    let result1 = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("payload"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(result1.is_success(), "first execution should succeed");
    assert_eq!(
        call_count.load(AOrdering::Relaxed),
        1,
        "handler should be called exactly once on first run"
    );

    // Reconstruct the idempotency key via the just-recorded
    // attempt history. T4 made
    // `ExecutionState::idempotency_key_for_node` return the key
    // for the **next** dispatch, so to read the key the engine
    // actually persisted, look at the last attempt record (which
    // carries the idempotency key it was minted under).
    //
    // Deserializing via a JSON string (rather than `from_value`)
    // avoids `#[serde(borrow)]` issues on domain keys — the same
    // workaround `resume_execution` applies when loading state.
    let execution_id = result1.execution_id;
    let (_, state_json) = stores
        .get_state(execution_id)
        .await
        .unwrap()
        .expect("execution state must be persisted after first run");
    let state_str = serde_json::to_string(&state_json).unwrap();
    let exec_state: ExecutionState =
        serde_json::from_str(&state_str).expect("deserialize persisted execution state");
    // The engine marks idempotency under the test scope with the
    // attempt number it dispatched the node under (1 on the first
    // run). Assert that mark was recorded without perturbing it.
    let attempt = exec_state
        .node_state(n.clone())
        .map(|ns| ns.attempts.len() as u32)
        .filter(|&a| a >= 1)
        .expect("first run must have pushed an attempt record ");

    let already_marked = stores.is_idempotency_marked(execution_id, n.clone(), attempt);
    assert!(
        already_marked,
        "idempotency key should be recorded after first execution"
    );

    // Also verify the persisted output is loadable.
    let persisted = stores.load_node_output(execution_id, n).await.unwrap();
    assert_eq!(
        persisted,
        Some(serde_json::json!("payload")),
        "persisted output should match the original execution result"
    );
}

// -- Action version pinning tests --

/// When `interface_version` is set on a node, the engine uses the versioned
/// handler instead of the latest.
#[tokio::test]
async fn version_pinned_node_uses_specified_handler() {
    use semver::Version;

    // V1 handler returns "v1".
    struct V1Handler;

    impl Action for V1Handler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("versioned.v1.static"), "V1", "v1 static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for V1Handler {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(serde_json::json!("v1")))
        }
    }

    // V2 handler returns "v2".
    struct V2Handler;

    impl Action for V2Handler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("versioned.v2.static"), "V2", "v2 static")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for V2Handler {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(serde_json::json!("v2")))
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    let v1 = Version::new(1, 0, 0);
    let v2 = Version::new(2, 0, 0);
    // Register v1 first; v2 will become the "latest" (handlers map entry).
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("versioned"), "V1", "v1 handler")
            .with_version_full(v1.clone()),
        V1Handler,
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("versioned"), "V2", "v2 handler")
            .with_version_full(v2.clone()),
        V2Handler,
    );

    let (engine, _) = make_engine(registry);

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");

    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "pinned_v1", "core", "versioned")
                .unwrap()
                .with_interface_version(v1),
            NodeDefinition::new(n2.clone(), "pinned_v2", "core", "versioned")
                .unwrap()
                .with_interface_version(v2),
        ],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(
        result.node_output(&n1),
        Some(&serde_json::json!("v1")),
        "n1 should use the v1 handler"
    );
    assert_eq!(
        result.node_output(&n2),
        Some(&serde_json::json!("v2")),
        "n2 should use the v2 handler"
    );
}

// -- Proactive credential refresh tests --

/// When a credential refresh hook is set, it is called before each node dispatch.
#[tokio::test]
async fn credential_refresh_hook_is_called_before_node_dispatch() {
    use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

    let refresh_count = Arc::new(AtomicU32::new(0));
    let refresh_count_clone = refresh_count.clone();

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    // The refresh hook is only called when a credential resolver is also set.
    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_credential_resolver(|_id: &str| async move {
            Err(nebula_credential::CredentialAccessError::NotFound(
                "no credentials".to_owned(),
            ))
        })
        .with_credential_refresh(move |_id: &str| {
            let count = refresh_count_clone.clone();
            async move {
                count.fetch_add(1, AOrdering::Relaxed);
                Ok(())
            }
        });

    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(n2.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![Connection::new(n1, n2)],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("x"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    // Two nodes → hook called twice.
    assert_eq!(
        refresh_count.load(AOrdering::Relaxed),
        2,
        "refresh hook should be called once per dispatched node"
    );
}

// -- Multi-edge regression tests --

/// Regression: two distinct edges from the same source to the same target
/// must not stall the target node.
///
/// Previously, `resolved_edges` used `HashSet<NodeKey>` (source-node cardinality)
/// while `required_count` counted edges. With two edges A → B, the set deduped
/// them to one entry, so `resolved(1) != required(2)` forever and B never ran.
/// The fix changes `resolved_edges` to `HashMap<NodeKey, usize>` (edge-count
/// cardinality), so both increments are counted and B correctly becomes ready.
#[tokio::test]
async fn multi_edge_from_same_source_executes_target() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );
    let (engine, _) = make_engine(registry);

    let a = node_key!("a");
    let b = node_key!("b");

    // Two distinct (non-identical) edges from A to B: one unconditional,
    // one via a named source port. Both activate on success.
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a, b.clone()).with_from_port(port_key!("alt")),
        ],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("payload"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(
        result.is_success(),
        "multi-edge workflow must complete successfully; got: {:?}",
        result.status
    );
    assert!(
        result.node_output(&b).is_some(),
        "target node B must execute and produce output"
    );
}

/// Regression for #341: `determine_final_status` must return `Failed`
/// (not `Completed`) when at least one node has not reached a terminal
/// state, even when no node explicitly failed and the cancellation token
/// is not set.
///
/// Additionally, it must attach an `integrity_violation` payload naming
/// the non-terminal nodes, so the caller can emit
/// `ExecutionEvent::FrontierIntegrityViolation` rather than silently
/// reporting success (PRODUCT_CANON ).
#[test]
fn final_status_guard_returns_failed_for_non_terminal_nodes() {
    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let n1 = node_key!("n1");
    let n2 = node_key!("n2");

    // n1 completed, n2 still Pending (simulates a stalled node).
    let mut exec_state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
    exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
    // n2 stays NodeState::Pending

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Failed,
        "non-terminal nodes must prevent a false Completed status"
    );
    let non_terminal = decision
        .integrity_violation
        .expect("integrity_violation must be populated when guard fires");
    assert_eq!(non_terminal.len(), 1, "exactly one node is non-terminal");
    assert_eq!(
        non_terminal[0],
        (n2, NodeState::Pending),
        "payload must name the stalled node and its observed state"
    );
}

/// Smoke-test: `determine_final_status` returns `Completed` when all nodes are
/// terminal and there is no failure or cancellation.
#[test]
fn final_status_completed_when_all_terminal() {
    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let n1 = node_key!("n1");
    let n2 = node_key!("n2");

    let mut exec_state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
    exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
    exec_state.node_states.get_mut(&n2).unwrap().state = NodeState::Skipped;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Completed,
        "all-terminal nodes with no failure must yield Completed"
    );
    assert!(
        decision.integrity_violation.is_none(),
        "no integrity payload when the invariant holds"
    );
}

/// Invariant: no combination of `(failed_node, cancel_token, exec_state)`
/// may produce `Completed` when `all_nodes_terminal` is false.
///
/// Acts as a lightweight property-style check — enumerates the cartesian
/// product of the three input axes for a two-node workflow and asserts
/// the frontier integrity (CAS on version) rule across every combination.
#[test]
fn final_status_never_completed_with_non_terminal_nodes() {
    use NodeState::*;
    let states = [
        Pending, Ready, Running, Completed, Failed, Skipped, Cancelled,
    ];
    let failure_cases = [None, Some((node_key!("n1"), "boom".to_owned()))];
    let cancel_cases = [false, true];

    let combinations = states
        .iter()
        .flat_map(|&a| std::iter::repeat(a).zip(states.iter().copied()))
        .flat_map(|(a, b)| failure_cases.iter().map(move |f| (a, b, f)))
        .flat_map(|(a, b, f)| cancel_cases.iter().map(move |&c| (a, b, f, c)));
    for (a, b, failed, cancel) in combinations {
        check_no_false_completed(a, b, failed, cancel);
    }
}

fn check_no_false_completed(
    a: NodeState,
    b: NodeState,
    failed: &Option<(NodeKey, String)>,
    cancel: bool,
) {
    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let mut state = ExecutionState::new(exec_id, wf_id, &[n1.clone(), n2.clone()]);
    state.node_states.get_mut(&n1).unwrap().state = a;
    state.node_states.get_mut(&n2).unwrap().state = b;

    let token = CancellationToken::new();
    if cancel {
        token.cancel();
    }

    let decision = determine_final_status(failed, &token, &state);
    if decision.status != ExecutionStatus::Completed {
        return;
    }
    assert!(
        state.all_nodes_terminal(),
        "Completed must imply all_nodes_terminal; \
             violated with a={a:?} b={b:?} failed={failed:?} cancel={cancel}"
    );
    assert!(
        decision.integrity_violation.is_none(),
        "Completed decisions must not carry an integrity payload"
    );
}

// ── ROADMAP §M0.3 — `determine_final_status` priority-ladder unit tests ──
//
// These cover the seven branches of the explicit-termination ladder
// documented on `determine_final_status`: explicit termination beats
// failed_node beats cancel_token beats integrity violation beats natural
// completion. Pairs with the engine integration tests that exercise
// explicit-termination behavior end-to-end (control_dispatch.rs,
// resource_integration.rs).

fn make_two_terminal_state(
    terminated_by: Option<(NodeKey, ExecutionTerminationReason)>,
) -> ExecutionState {
    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let mut state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        &[n1.clone(), n2.clone()],
    );
    // Drive both nodes to a terminal state so the integrity guard
    // does not fire for tests that do not want to exercise it.
    state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
    state.node_states.get_mut(&n2).unwrap().state = NodeState::Skipped;
    state.terminated_by = terminated_by;
    state
}

/// Priority 1 (Stop): explicit-stop signal yields `Completed` plus the
/// `ExplicitStop` reason regardless of natural drainage.
#[test]
fn final_status_explicit_stop_yields_completed_with_explicit_reason() {
    let n1 = node_key!("n1");
    let reason = ExecutionTerminationReason::ExplicitStop {
        by_node: n1.clone(),
        note: Some("done".to_owned()),
    };
    let state = make_two_terminal_state(Some((n1, reason.clone())));

    let token = CancellationToken::new();
    let decision = determine_final_status(&None, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Completed);
    assert_eq!(decision.termination_reason, Some(reason));
    assert!(decision.integrity_violation.is_none());
}

/// Priority 1 (Fail): explicit-fail signal yields `Failed` plus the
/// `ExplicitFail` reason — distinct from a system-driven `Failed`.
#[test]
fn final_status_explicit_fail_yields_failed_with_explicit_reason() {
    let n1 = node_key!("n1");
    let reason = ExecutionTerminationReason::ExplicitFail {
        by_node: n1.clone(),
        code: nebula_execution::status::ExecutionTerminationCode::new("E_FAIL"),
        message: "boom".to_owned(),
    };
    let state = make_two_terminal_state(Some((n1, reason.clone())));

    let token = CancellationToken::new();
    let decision = determine_final_status(&None, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Failed);
    assert_eq!(decision.termination_reason, Some(reason));
}

/// Priority 2: a system-driven `failed_node` without an explicit
/// termination yields `(Failed, None)` — the `None` is load-bearing
/// (signals "engine has nothing extra to attribute").
#[test]
fn final_status_failed_node_without_terminate_yields_failed_none() {
    let state = make_two_terminal_state(None);
    let token = CancellationToken::new();
    let failed = Some((node_key!("n1"), "boom".to_owned()));

    let decision = determine_final_status(&failed, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Failed);
    assert!(decision.termination_reason.is_none());
}

/// Priority 3: external cancel without an explicit termination yields
/// `(Cancelled, Cancelled)` — distinct from explicit-stop.
#[test]
fn final_status_external_cancel_yields_cancelled_with_cancelled_reason() {
    let state = make_two_terminal_state(None);
    let token = CancellationToken::new();
    token.cancel();

    let decision = determine_final_status(&None, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Cancelled);
    assert_eq!(
        decision.termination_reason,
        Some(ExecutionTerminationReason::Cancelled)
    );
}

/// Priority 5: natural drainage with all-terminal nodes and no signal
/// yields `(Completed, NaturalCompletion)`.
#[test]
fn final_status_natural_completion_yields_completed_with_natural_reason() {
    let state = make_two_terminal_state(None);
    let token = CancellationToken::new();

    let decision = determine_final_status(&None, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Completed);
    assert_eq!(
        decision.termination_reason,
        Some(ExecutionTerminationReason::NaturalCompletion)
    );
}

/// Priority 1 wins over Priority 2: explicit stop authoritative even
/// when a sibling failed mid-cancel. The user's stop signal is
/// authoritative; sibling failure is collateral.
#[test]
fn final_status_explicit_stop_wins_over_failed_node() {
    let n1 = node_key!("n1");
    let stop_reason = ExecutionTerminationReason::ExplicitStop {
        by_node: n1.clone(),
        note: None,
    };
    let state = make_two_terminal_state(Some((n1, stop_reason.clone())));
    let token = CancellationToken::new();
    // Sibling failure that would have promoted to Failed under priority 2.
    let failed = Some((node_key!("n2"), "sibling exploded mid-cancel".to_owned()));

    let decision = determine_final_status(&failed, &token, &state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Completed,
        "ExplicitStop must win over sibling failure"
    );
    assert_eq!(decision.termination_reason, Some(stop_reason));
}

/// Priority 1 wins over Priority 2 (Fail variant): an explicit fail
/// signal is authoritative even when a sibling also failed.
#[test]
fn final_status_explicit_fail_wins_over_failed_sibling() {
    let n1 = node_key!("n1");
    let fail_reason = ExecutionTerminationReason::ExplicitFail {
        by_node: n1.clone(),
        code: nebula_execution::status::ExecutionTerminationCode::new("E_USER_FAIL"),
        message: "user-driven".to_owned(),
    };
    let state = make_two_terminal_state(Some((n1, fail_reason.clone())));
    let token = CancellationToken::new();
    let failed = Some((node_key!("n2"), "sibling crash".to_owned()));

    let decision = determine_final_status(&failed, &token, &state);

    assert_eq!(decision.status, ExecutionStatus::Failed);
    assert_eq!(
        decision.termination_reason,
        Some(fail_reason),
        "ExplicitFail must win over sibling failure"
    );
}

/// Regression for #341: when the guard populates a non-terminal payload,
/// `emit_frontier_integrity_if_violated` must send exactly one
/// `ExecutionEvent::FrontierIntegrityViolation`. Covers the helper all
/// three finish sites call, so a reorder or drop at any site is caught
/// centrally.
#[tokio::test]
async fn emit_frontier_integrity_helper_delivers_one_event_on_violation() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(8);
    let mut rx = event_bus.subscribe();
    let engine = engine.with_event_bus(event_bus);

    let exec_id = ExecutionId::new();
    let n2 = node_key!("n2");
    let payload = Some(vec![(n2.clone(), NodeState::Pending)]);
    engine.emit_frontier_integrity_if_violated(exec_id, payload);

    match rx.try_recv().expect("violation event") {
        ExecutionEvent::FrontierIntegrityViolation {
            execution_id,
            non_terminal_nodes,
        } => {
            assert_eq!(execution_id, exec_id);
            assert_eq!(non_terminal_nodes, vec![(n2, NodeState::Pending)]);
        },
        other => panic!("expected FrontierIntegrityViolation, got {other:?}"),
    }
    // No further events from this helper — the finish event is the
    // caller's responsibility and is intentionally out of scope here.
    assert!(
        rx.try_recv().is_none(),
        "helper must emit exactly one event"
    );
}

/// When the guard does not fire, `emit_frontier_integrity_if_violated`
/// must stay silent so the finish-event stream is unchanged in the
/// happy path.
#[tokio::test]
async fn emit_frontier_integrity_helper_silent_when_no_violation() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);
    let event_bus = nebula_eventbus::EventBus::<ExecutionEvent>::new(8);
    let mut rx = event_bus.subscribe();
    let engine = engine.with_event_bus(event_bus);

    engine.emit_frontier_integrity_if_violated(ExecutionId::new(), None);
    assert!(rx.try_recv().is_none());
}

/// Regression for #306: when the proactive credential-refresh hook
/// returns an error, the node MUST end up Failed (not Completed) and
/// the failure MUST surface as a typed
/// `ActionError::CredentialRefreshFailed`, not a log-and-continue WARN.
///
/// Verifies:
///   1. The action handler is **never** invoked (refresh fails before dispatch).
///   2. The execution result is not a success.
///   3. The persisted `node_errors` string contains "credential refresh failed"
///      and the original source message, confirming the typed error propagated
///      through to the durable error record visible to downstream consumers.
///   4. The `EngineError::Action(CredentialRefreshFailed)` variant round-trips
///      through pattern-match correctly and is classified as retryable.
#[tokio::test]
async fn credential_refresh_failure_surfaces_as_typed_error() {
    use std::sync::atomic::{AtomicU32, Ordering as AOrdering};

    // Action that asserts it never runs — if the engine reaches
    // dispatch despite a failed refresh, this will fire and surface
    // a different error than the one we expect.
    struct NeverRunHandler {
        invoked: Arc<AtomicU32>,
    }
    impl Action for NeverRunHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("never.static"), "Never", "must not run")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }
    impl StatelessAction for NeverRunHandler {
        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            self.invoked.fetch_add(1, AOrdering::Relaxed);
            Ok(ActionResult::success(input))
        }
    }

    let invoked = Arc::new(AtomicU32::new(0));
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("never"), "Never", "must not run"),
        NeverRunHandler {
            invoked: invoked.clone(),
        },
    );

    // Refresh hook always fails. Use `ActionError::retryable` for
    // the inner source so the `Arc<dyn Error>` wrapping in
    // `CredentialRefreshFailed` round-trips through Display.
    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_credential_resolver(|_id: &str| async move {
            Err(nebula_credential::CredentialAccessError::NotFound(
                "no credentials".to_owned(),
            ))
        })
        .with_credential_refresh(|_id: &str| async move {
            Err(ActionError::retryable("credential store down"))
        });

    let n1 = node_key!("n1");
    let wf = make_workflow(
        vec![NodeDefinition::new(n1.clone(), "A", "core", "never").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("x"),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult) even on node failure");

    // (1) The action body must NEVER have been called.
    assert_eq!(
        invoked.load(AOrdering::Relaxed),
        0,
        "action body must not run when proactive refresh fails"
    );

    // (2) Execution must NOT be a success.
    assert!(
        !result.is_success(),
        "workflow must not succeed when refresh fails"
    );

    // (3) The node-level error message must mention the typed cause.
    // The default ErrorStrategy is FailFast, so the engine populates
    // `node_errors` with the failed node's error message. The string
    // representation of the new variant is stable and downstream
    // consumers (TUI, log scrape, dashboards) can match on it.
    let node_err = result
        .node_errors
        .get(&n1)
        .expect("node_errors must contain the failed node");
    assert!(
        node_err.contains("credential refresh failed"),
        "expected typed CredentialRefreshFailed in error, got: {node_err}"
    );
    assert!(
        node_err.contains("credential store down"),
        "expected source string preserved in error, got: {node_err}"
    );

    // (4) Construct the variant directly and confirm classifier
    // routing — this is the contract downstream consumers match on.
    let typed =
        ActionError::credential_refresh_failed("never", ActionError::retryable("store down"));
    assert!(matches!(typed, ActionError::CredentialRefreshFailed { .. }));
    assert!(typed.is_retryable(), "default classification is retryable");
    let engine_err = EngineError::Action(typed);
    assert!(matches!(
        engine_err,
        EngineError::Action(ActionError::CredentialRefreshFailed { .. })
    ));
}

/// `error_is_terminal` decides retry eligibility by error nature (Codex P2).
/// Non-retryable runtime conditions are terminal; `AgentTurnTimeout` and
/// retryable action errors are not. Regression guard: the
/// `RuntimeError::ActionError` classify metadata (`retryable = false`) must
/// NOT shadow the inner action error's real retryability.
#[test]
fn error_is_terminal_classification() {
    use crate::runtime::RuntimeError;

    // Non-retryable runtime conditions → terminal (never re-dispatched).
    assert!(error_is_terminal(&EngineError::Runtime(
        RuntimeError::AgentBudgetExceeded {
            key: "a".into(),
            max_turns: 3,
        }
    )));
    assert!(error_is_terminal(&EngineError::Runtime(
        RuntimeError::AgentWaitNotSupported { key: "a".into() }
    )));

    // `AgentTurnTimeout` is retryable → NOT terminal.
    assert!(!error_is_terminal(&EngineError::Runtime(
        RuntimeError::AgentTurnTimeout {
            key: "a".into(),
            turn: 0,
            timeout: std::time::Duration::from_millis(10),
        }
    )));

    // Regression guard: a retryable action error (direct or runtime-wrapped)
    // must NOT be terminal, despite the wrapper variant's classify metadata.
    assert!(!error_is_terminal(&EngineError::Action(
        ActionError::retryable("x")
    )));
    assert!(!error_is_terminal(&EngineError::Runtime(
        RuntimeError::ActionError(ActionError::retryable("x"))
    )));

    // Fatal action errors are terminal.
    assert!(error_is_terminal(&EngineError::Action(ActionError::fatal(
        "x"
    ))));
}

// -- Credential allowlist enforcement (PRODUCT_CANON / — audit ) --

/// Stateless action that acquires a credential by id and records the result.
///
/// Used by the allowlist tests: the `credential_id` input parameter selects
/// which credential to probe. Registered via
/// [`ActionRegistry::register_stateless_instance`] with per-probe metadata so
/// distinct keys/names can be assigned without multiple struct definitions.
///
/// The outcome (success vs typed error) is surfaced via the execution result so
/// tests can assert that denial propagates as a real `ActionError` rather than
/// silently succeeding.
struct CredProbeAction;

impl Action for CredProbeAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        // Static placeholder — per-probe key/name are supplied at registration
        // via `register_stateless_instance(meta, CredProbeAction)`.
        ActionMetadata::new(
            action_key!("test.cred_probe"),
            "CredProbe",
            "acquires a credential",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for CredProbeAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let id = input
            .get("credential_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActionError::fatal("missing credential_id"))?;
        // `credential_by_id` forwards `CredentialAccessError::AccessDenied`
        // as `ActionError::CapabilityViolation` via the From impl in
        // `nebula_action::error`. We want the typed error to bubble up so
        // the engine records a NodeFailed — not to swallow it.
        let _snapshot = ctx.credential_by_id(id).await?;
        Ok(ActionResult::success(serde_json::json!({"ok": true})))
    }
}

/// Register a `CredProbeAction` under `key` into the given registry.
fn register_probe(registry: &ActionRegistry, key: ActionKey, name: &str) {
    let meta = ActionMetadata::new(key, name, "acquires a credential");
    registry.register_stateless_instance(meta, CredProbeAction);
}

/// Build a workflow with a single `CredProbeAction` node that probes `cred_id`.
fn probe_workflow(action: &str, cred_id: &str) -> WorkflowDefinition {
    let n1 = node_key!("probe");
    let node = NodeDefinition::new(n1, "probe", "core", action)
        .unwrap()
        .with_parameter(
            "credential_id",
            nebula_workflow::ParamValue::literal(serde_json::json!(cred_id)),
        );
    make_workflow(vec![node], vec![])
}

/// Build a snapshot the resolver can return for any id. Used to prove that
/// denial happens **before** the resolver is consulted, not as a side effect
/// of the store returning nothing — deny-by-default must be a real policy
/// check, not a lucky miss.
fn dummy_snapshot(id: &str) -> nebula_credential::CredentialSnapshot {
    nebula_credential::CredentialSnapshot::new(
        id,
        nebula_credential::CredentialRecord::new(),
        nebula_credential::SecretToken::new(nebula_credential::SecretString::new("test-value")),
    )
}

/// Default-deny: an action that was never declared to the engine cannot
/// acquire any credential — even one the resolver would happily return.
#[tokio::test]
async fn credential_access_denied_without_declaration() {
    let registry = Arc::new(ActionRegistry::new());
    register_probe(&registry, action_key!("probe"), "Probe");

    let (engine, _) = make_engine(registry);
    // No `with_action_credentials` — `probe` has no declaration.
    let engine = engine.with_credential_resolver(|id: &str| {
        let id = id.to_owned();
        async move { Ok(dummy_snapshot(&id)) }
    });

    let wf = probe_workflow("probe", "api_key");
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult) even on node failure");

    assert!(
        !result.is_success(),
        "undeclared action must not acquire credentials"
    );
    let err = result
        .node_errors
        .get(&node_key!("probe"))
        .expect("failed node must carry an error message");
    // `CredentialAccessError::AccessDenied` is mapped to
    // `ActionError::CapabilityViolation { capability, action_id }` (see
    // `nebula_action::error::From<CredentialAccessError>`), whose Display
    // is `"capability violation: capability `{capability}` denied for ..."`.
    assert!(
        err.contains("capability violation") && err.contains("denied"),
        "error must surface capability-violation denial, got: {err}"
    );
    assert!(
        err.contains("credential:api_key"),
        "error must attribute the denied credential id, got: {err}"
    );
    assert!(
        err.contains("for action `probe`"),
        "error must attribute the action whose access was denied, got: {err}"
    );
}

/// Declared: the engine permits exactly the credential ids explicitly
/// declared for the action's `ActionKey`.
#[tokio::test]
async fn credential_access_allowed_with_declaration() {
    let registry = Arc::new(ActionRegistry::new());
    register_probe(&registry, action_key!("probe"), "Probe");

    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_credential_resolver(|id: &str| {
            let id = id.to_owned();
            async move { Ok(dummy_snapshot(&id)) }
        })
        .with_action_credentials(action_key!("probe"), ["api_key"]);

    let wf = probe_workflow("probe", "api_key");
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult)");

    assert!(
        result.is_success(),
        "declared credential must be acquirable, errors: {:?}",
        result.node_errors
    );
}

/// Mismatched: an action that declares credential `A` still cannot acquire
/// credential `B`. Per-key enforcement, not per-action blanket allow.
#[tokio::test]
async fn credential_access_denied_for_mismatched_key() {
    let registry = Arc::new(ActionRegistry::new());
    register_probe(&registry, action_key!("probe"), "Probe");

    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_credential_resolver(|id: &str| {
            let id = id.to_owned();
            async move { Ok(dummy_snapshot(&id)) }
        })
        .with_action_credentials(action_key!("probe"), ["cred_a"]);

    let wf = probe_workflow("probe", "cred_b");
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult) even on node failure");

    assert!(
        !result.is_success(),
        "mismatched credential id must not be acquirable"
    );
    let err = result
        .node_errors
        .get(&node_key!("probe"))
        .expect("failed node must carry an error message");
    assert!(
        err.contains("capability violation") && err.contains("denied"),
        "error must surface capability-violation denial, got: {err}"
    );
    assert!(
        err.contains("credential:cred_b"),
        "error must attribute the denied credential id (cred_b), got: {err}"
    );
}

/// Scoping: declarations for one `ActionKey` do not leak to others.
#[tokio::test]
async fn credential_declaration_is_per_action_key() {
    let registry = Arc::new(ActionRegistry::new());
    register_probe(&registry, action_key!("probe_a"), "Probe A");
    register_probe(&registry, action_key!("probe_b"), "Probe B");

    let (engine, _) = make_engine(registry);
    let engine = engine
            .with_credential_resolver(|id: &str| {
                let id = id.to_owned();
                async move { Ok(dummy_snapshot(&id)) }
            })
            // Only `probe_a` declares `shared_key`. `probe_b` must still be denied.
            .with_action_credentials(action_key!("probe_a"), ["shared_key"]);

    // probe_b tries shared_key → must fail even though probe_a has it declared.
    let wf = probe_workflow("probe_b", "shared_key");
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult)");

    assert!(
        !result.is_success(),
        "declaration for probe_a must not leak to probe_b"
    );
}

/// Merging: repeat declarations for the same `ActionKey` add keys cumulatively
/// rather than replacing the set.
#[tokio::test]
async fn action_credentials_merge_across_builder_calls() {
    let registry = Arc::new(ActionRegistry::new());
    register_probe(&registry, action_key!("probe"), "Probe");

    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_credential_resolver(|id: &str| {
            let id = id.to_owned();
            async move { Ok(dummy_snapshot(&id)) }
        })
        .with_action_credentials(action_key!("probe"), ["first"])
        .with_action_credentials(action_key!("probe"), ["second"]);

    // Probing "second" must succeed — the second call adds, not replaces.
    let wf = probe_workflow("probe", "second");
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("engine returns Ok(ExecutionResult)");
    assert!(
        result.is_success(),
        "repeated with_action_credentials must merge, not replace. errors: {:?}",
        result.node_errors
    );
}

// -- Regression tests for batch 2 (#299, #300, #301, #311, #321) --

/// Issue #321 — the setup-failure path (parameter resolution error,
/// missing node definition, invalid state-machine start) must
/// checkpoint the execution state, symmetrical with the runtime-
/// failure path. Previously only the runtime branch checkpointed,
/// so a setup failure left the persisted state describing the node
/// as Pending even though it was Failed in memory.
#[tokio::test]
async fn setup_failure_checkpoints_execution_state() {
    // Force parameter resolution to fail by referencing a node
    // that has no output in the shared outputs map.
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = engine.with_execution_stores(stores.execution_stores());

    let n1 = node_key!("n1");
    let ghost = node_key!("ghost");
    let mut params: HashMap<String, nebula_workflow::ParamValue> = HashMap::new();
    params.insert(
        "input".into(),
        nebula_workflow::ParamValue::Reference {
            node_key: ghost,
            output_path: String::new(),
        },
    );
    let mut node = NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap();
    node.parameters = params;

    let wf = make_workflow(vec![node], vec![]);

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("hello"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure(), "setup failure should fail execution");

    // The critical assertion: the execution state was checkpointed
    // after the setup failure. The persisted status must be
    // `failed` and the failed node's state must be `failed` with
    // an error_message populated.
    let (_version, state_json) = stores
        .get_state(result.execution_id)
        .await
        .unwrap()
        .expect("execution state should be persisted after setup failure");
    assert_eq!(
        state_json.get("status").and_then(|s| s.as_str()),
        Some("failed"),
        "execution status should be persisted as failed"
    );
    let node_state = state_json
        .pointer(&format!("/node_states/{n1}/state"))
        .and_then(|v| v.as_str());
    assert_eq!(
        node_state,
        Some("failed"),
        "node state should be persisted as failed after setup failure (issue #321)"
    );
    let err_msg = state_json
        .pointer(&format!("/node_states/{n1}/error_message"))
        .and_then(|v| v.as_str());
    assert!(
        err_msg.is_some(),
        "setup-failure error message should be persisted, got state: {state_json}"
    );
}

/// Issue #311 — resume_execution must restore the original
/// workflow input from the persisted state, not substitute Null.
/// Regression: `ExecutionState::workflow_input` is now persisted
/// at execution start and read back on resume.
#[tokio::test]
async fn resume_restores_original_workflow_input() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);

    let n1 = node_key!("n1");
    let wf = make_workflow(
        vec![NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;

    // Build a partial execution state for a FRESH execution where
    // the entry node has not yet run. Persist it with the original
    // trigger payload set via `set_workflow_input`.
    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
    exec_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    exec_state.set_workflow_input(serde_json::json!({"trigger": "webhook-payload"}));
    let state_json = serde_json::to_value(&exec_state).unwrap();
    stores.inject_state(execution_id, wf.id, state_json).await;

    let engine = stores.attach(engine);

    let scope = crate::store_seam::single_tenant_scope();
    let result = engine.resume_execution(&scope, execution_id).await.unwrap();

    assert!(result.is_success());
    // Echo pipes the input through — so n1's output is exactly
    // the workflow input the engine restored from storage.
    assert_eq!(
        result.node_output(&n1),
        Some(&serde_json::json!({"trigger": "webhook-payload"})),
        "resume should feed the entry node the persisted trigger payload, not Null (issue #311)"
    );
}

/// Issue #289 — `resume_execution` must restore the persisted
/// `ExecutionBudget` instead of silently reverting to
/// `ExecutionBudget::default()`. Before the fix, a run configured
/// with a tight concurrency / retry / timeout budget would resume
/// with the default 10-way concurrency and unbounded retries,
/// changing behavior vs operator expectations. See
/// `PRODUCT_CANON.md ` (public surface honored end-to-end).
#[tokio::test]
async fn resume_restores_persisted_budget() {
    use std::time::Duration;

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    // Start an execution on "engine A" with a non-default budget
    // (max_concurrent_nodes=3 + retries + timeout + output cap),
    // persist it, then resume on a fresh "engine B" that has never
    // seen the budget in-memory. The persisted row is the only
    // channel for the budget to reach the resumed run.
    let stores = TestStores::new();
    let n1 = node_key!("n1");
    let wf = make_workflow(
        vec![NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;

    let configured = ExecutionBudget::default()
        .with_max_concurrent_nodes(3)
        .with_max_duration(Duration::from_secs(97))
        .with_max_output_bytes(7 * 1024);

    // Simulate the state "engine A" would have written right
    // after `execute_workflow` began but before the node ran:
    // status=Running, entry node still Pending, budget persisted.
    // This mirrors the real crash window the fix covers (the
    // setup-failure / post-create checkpoint).
    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
    exec_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    exec_state.set_budget(configured.clone());
    let state_json = serde_json::to_value(&exec_state).unwrap();
    stores.inject_state(execution_id, wf.id, state_json).await;

    // Resume on a fresh engine ("engine B" — new runner, new
    // instance, no memory of the original budget).
    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);
    let scope = crate::store_seam::single_tenant_scope();
    let result = engine.resume_execution(&scope, execution_id).await.unwrap();
    assert!(result.is_success());

    // Re-load the persisted state and assert the budget survived
    // the resume unchanged — this proves the resume path reads
    // the budget off the row rather than substituting a default.
    //
    // Deserialize via a JSON string (not `serde_json::from_value`)
    // because `ExecutionState::node_states` uses `NodeKey` which
    // has a borrowed-string `Deserialize` impl incompatible with
    // `from_value` (docs/pitfalls — serde MapAccess).
    let (_v, state_after) = stores.get_state(execution_id).await.unwrap().unwrap();
    let state_after_str = serde_json::to_string(&state_after).unwrap();
    let round_tripped: ExecutionState = serde_json::from_str(&state_after_str).unwrap();
    let restored = round_tripped
        .budget
        .expect("resume must preserve the persisted budget on the execution row");
    assert_eq!(
        restored, configured,
        "resume must use the persisted budget, not ExecutionBudget::default() (issue #289)"
    );
    // And specifically NOT the default — guards against a silent
    // regression where the code accidentally overwrites the field
    // with `default()` before the final persist.
    assert_ne!(
        restored,
        ExecutionBudget::default(),
        "the configured budget must not collapse to default() on resume"
    );
}

/// Issue #289 — legacy persisted states that predate budget
/// persistence must still resume (falling back to
/// `ExecutionBudget::default()` with a warning log), so the fix
/// does not break old rows.
#[tokio::test]
async fn resume_falls_back_to_default_budget_on_legacy_state() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let n1 = node_key!("n1");
    let wf = make_workflow(
        vec![NodeDefinition::new(n1.clone(), "A", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;

    // Build a state snapshot with NO `budget` field — simulates a
    // pre-#289 row. We build the state normally, serialize it,
    // then strip the field before persist so the resume path
    // observes it as `None` (the legacy deserialization outcome).
    let execution_id = ExecutionId::new();
    let mut exec_state = ExecutionState::new(execution_id, wf.id, std::slice::from_ref(&n1));
    exec_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    // Don't set the budget. Confirm the field is absent after
    // roundtrip — catches a future default change that would
    // accidentally inject a value.
    assert!(exec_state.budget.is_none());
    let mut state_json = serde_json::to_value(&exec_state).unwrap();
    if let Some(obj) = state_json.as_object_mut() {
        obj.remove("budget");
    }
    stores.inject_state(execution_id, wf.id, state_json).await;

    let (engine, _) = make_engine(registry);
    let engine = stores.attach(engine);

    // Resume must succeed despite the missing budget — the engine
    // logs a warning and falls back to the default.
    let scope = crate::store_seam::single_tenant_scope();
    let result = engine.resume_execution(&scope, execution_id).await.unwrap();
    assert!(result.is_success());
}

/// Issue #300 — spawn_node must NOT silently spawn a task on a
/// node whose state machine cannot reach Running from its current
/// position. When the engine is asked to spawn a node that is
/// already Completed (e.g. via a manually-manipulated state), the
/// typed `start_node_attempt` helper rejects the transition and
/// the node is routed through the setup-failure path.
#[test]
fn start_node_attempt_rejects_terminal_state() {
    let n1 = node_key!("n1");
    let mut state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        std::slice::from_ref(&n1),
    );
    // Drive n1 to Completed via the legal transition chain.
    state.transition_node(n1.clone(), NodeState::Ready).unwrap();
    state
        .transition_node(n1.clone(), NodeState::Running)
        .unwrap();
    state
        .transition_node(n1.clone(), NodeState::Completed)
        .unwrap();

    let err = state
        .start_node_attempt(n1.clone())
        .expect_err("start_node_attempt must reject Completed source state");
    assert!(
        err.to_string().contains("invalid transition"),
        "error should be InvalidTransition, got: {err}"
    );
    // State must not have moved.
    assert_eq!(state.node_state(n1).unwrap().state, NodeState::Completed);
}

/// Issue #301 — when a node task panics, the engine must report
/// the real NodeKey, not a synthesized placeholder. Regression
/// verified via a panicking handler.
#[tokio::test]
async fn panicked_task_reports_real_node_id() {
    struct PanicHandler;

    impl Action for PanicHandler {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(action_key!("boom.static"), "Boom", "panics")
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for PanicHandler {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            panic!("intentional panic for test");
        }
    }

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("boom"), "Boom", "panics"),
        PanicHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = engine.with_execution_stores(stores.execution_stores());

    let n1 = node_key!("n1");
    let wf = make_workflow(
        vec![NodeDefinition::new(n1.clone(), "Boom", "core", "boom").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("ignored"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_failure(), "panicked workflow must fail");

    // The node errors map must list the real n1 key with a
    // non-empty message, not some synthetic NodeKey.
    let err_msg = result
        .node_errors
        .get(&n1)
        .expect("panicked node must be recorded under its real NodeKey (issue #301)");
    assert!(
        !err_msg.is_empty(),
        "panic error message should not be empty, got: {err_msg:?}"
    );

    // Persisted state should also reflect n1 as the failed node.
    let (_v, state_json) = stores
        .get_state(result.execution_id)
        .await
        .unwrap()
        .expect("state persisted after panic");
    let node_state = state_json
        .pointer(&format!("/node_states/{n1}/state"))
        .and_then(|v| v.as_str());
    assert_eq!(
        node_state,
        Some("failed"),
        "panicked node should be checkpointed as Failed"
    );
}

/// Issue #299 — idempotency replay must reconstruct the exact
/// ActionResult variant so that Branch edges gate correctly.
/// Regression: with the old code a persisted Branch result was
/// replayed as a flat `Success`, and every branch edge fired
/// regardless of `branch_key`, causing unintended downstream
/// execution on replay.
#[tokio::test]
async fn idempotency_replay_preserves_branch_routing() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("branch"), "Branch", "branches"),
        BranchHandler {
            selected: nebula_action::branch_key!("true"),
        },
    );
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let engine = engine.with_execution_stores(stores.execution_stores());

    // A → B (branch_key="true") / C (branch_key="false")
    let a = node_key!("a");
    let b = node_key!("b");
    let c = node_key!("c");
    let wf = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "branch").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(c.clone(), "C", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()).with_from_port(port_key!("true")),
            Connection::new(a.clone(), c.clone()).with_from_port(port_key!("false")),
        ],
    );

    // First run: A emits Branch{selected=true}. Only B fires.
    let first = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("payload"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(first.is_success());
    assert!(
        first.node_output(&b).is_some(),
        "B should run on first pass"
    );
    assert!(
        first.node_output(&c).is_none(),
        "C should NOT run on first pass (false branch)"
    );

    // Verify the persisted ActionResult encodes a Branch variant
    // rather than bare output — this is the byte-level check
    // behind issue #299's fix.
    let persisted_record = stores
        .load_node_result(first.execution_id, a.clone())
        .await
        .unwrap()
        .expect("load_node_result should return the persisted ActionResult after #299");
    assert_eq!(
        persisted_record.kind_tag, "Branch",
        "persisted ActionResult for A should be the Branch variant, got: {persisted_record:?}"
    );
    assert_eq!(
        persisted_record
            .json
            .get("selected")
            .and_then(|v| v.as_str()),
        Some("true"),
        "Branch selector should be persisted verbatim"
    );
}

// -- Regression tests for #333 (CAS-conflict reconciliation) --

/// Wraps an inner [`ExecutionStore`] and injects a single external
/// "concurrent" commit BEFORE the Nth `commit()` call from the engine
/// — bumping the version and optionally rewriting the status to
/// simulate an API cancel / admin mutation / sibling runner.
/// Subsequent engine commits hit a version mismatch.
///
/// The injected commit reuses the row's current fencing generation
/// (read via `get`), so it is *not* a fencing race — it is a pure CAS
/// race, exactly the pre-fix #333 failure mode where the engine
/// silently overwrote external state on a version mismatch.
#[derive(Debug)]
struct ExternalMutateBeforeN {
    inner: Arc<nebula_storage::InMemoryExecutionStore>,
    mutate_before: u32,
    new_status: Option<String>,
    calls: AtomicU32,
    injected: std::sync::atomic::AtomicBool,
}

impl ExternalMutateBeforeN {
    fn new(
        inner: Arc<nebula_storage::InMemoryExecutionStore>,
        mutate_before: u32,
        new_status: Option<&str>,
    ) -> Self {
        Self {
            inner,
            mutate_before,
            new_status: new_status.map(ToOwned::to_owned),
            calls: AtomicU32::new(0),
            injected: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionStore for ExternalMutateBeforeN {
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

    async fn get(
        &self,
        scope: &Scope,
        id: &str,
    ) -> Result<Option<nebula_storage_port::dto::ExecutionRecord>, StorageError> {
        self.inner.get(scope, id).await
    }

    async fn commit(
        &self,
        batch: nebula_storage_port::TransitionBatch,
    ) -> Result<nebula_storage_port::TransitionOutcome, StorageError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.mutate_before
            && !self.injected.swap(true, Ordering::SeqCst)
            && let Ok(Some(record)) = self.inner.get(batch.scope(), batch.execution_id()).await
        {
            let mut state = record.state.clone();
            if let Some(status) = &self.new_status
                && let Some(obj) = state.as_object_mut()
            {
                obj.insert(
                    "status".to_owned(),
                    serde_json::Value::String(status.clone()),
                );
            }
            // External commit at the version the engine believes is
            // current, reusing the live fencing generation so this is
            // a pure CAS race (not a fencing race). Bumps the version
            // beneath the engine's feet.
            if let Ok(external) = nebula_storage_port::TransitionBatch::builder()
                .scope(record.scope.clone())
                .execution_id(record.id.clone())
                .expected_version(record.version)
                .fencing(nebula_storage_port::FencingToken::from_generation(
                    record.fencing.unwrap_or(0),
                ))
                .new_state(state)
                .build()
            {
                let _ = self.inner.commit(external).await;
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
    ) -> Result<Option<nebula_storage_port::FencingToken>, StorageError> {
        self.inner.acquire_lease(scope, id, holder, ttl).await
    }

    async fn renew_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: nebula_storage_port::FencingToken,
        ttl: Duration,
    ) -> Result<bool, StorageError> {
        self.inner.renew_lease(scope, id, token, ttl).await
    }

    async fn release_lease(
        &self,
        scope: &Scope,
        id: &str,
        token: nebula_storage_port::FencingToken,
    ) -> Result<bool, StorageError> {
        self.inner.release_lease(scope, id, token).await
    }

    async fn list_all_running(
        &self,
    ) -> Result<Vec<nebula_storage_port::dto::ExecutionRecord>, StorageError> {
        self.inner.list_all_running().await
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

/// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
///
/// When the engine's final `transition()` CAS-misses because an
/// external actor (API cancel, admin mutation, sibling runner)
/// committed a **terminal** transition first, the engine MUST
/// honor the external terminal state rather than overwrite it.
/// Pre-fix, the engine only refreshed the version and continued
/// reporting its local `Completed` status while the persisted row
/// carried (say) `Cancelled` — a silent overwrite of concurrent
/// state. With the fix, `persist_final_state` detects the terminal
/// external status and surfaces it in the `ExecutionResult`.
#[tokio::test]
async fn final_cas_conflict_with_external_cancel_honors_external_status() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        EchoHandler,
    );

    let a = node_key!("a");
    let wf = make_workflow(
        vec![NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap()],
        vec![],
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    // `create()` seeds version=0 without a commit() call. The engine
    // then issues two commit() calls: #1 is the node checkpoint
    // (v=0 → v=1) and #2 is the final state write (v=1 → v=2).
    // Inject the external mutation before call #2 so the FINAL CAS
    // misses.
    let inner = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mutating = Arc::new(ExternalMutateBeforeN::new(
        inner.clone(),
        2,
        Some("cancelled"),
    ));
    let execution_stores = crate::store_seam::ExecutionStores {
        execution: mutating,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(inner.resume_token_store()),
    };

    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_execution_stores(execution_stores)
        .with_workflow_stores(stores.workflow_stores());

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow should return Ok on external terminal override (§11.5, #333)");

    assert_eq!(
        result.status,
        ExecutionStatus::Cancelled,
        "engine must surface the external Cancelled status when its \
             final CAS collides with a terminal external transition \
             (pre-fix it reported Completed, silently overwriting the \
             concurrent cancel — §11.5, #333). got {:?}",
        result.status
    );

    // The persisted row must carry `cancelled` — the engine must
    // NOT have overwritten it with its own `completed`.
    let scope = crate::store_seam::single_tenant_scope();
    let record = inner
        .get(&scope, &result.execution_id.to_string())
        .await
        .unwrap()
        .expect("persisted state must exist");
    assert_eq!(
        record.state.get("status").and_then(|v| v.as_str()),
        Some("cancelled"),
        "persisted row must retain the external Cancelled status; \
             engine must not overwrite a concurrent terminal transition \
             (§11.5, #333)"
    );
}

/// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
///
/// On `checkpoint_node` CAS mismatch, the engine now returns the
/// typed [`EngineError::CasConflict`] carrying the observer-visible
/// external status — not a generic `CheckpointFailed`. Pre-fix,
/// only the version was refreshed (the observed state was
/// discarded) and the error reason was a bare string, leaving no
/// structured signal for operators or upstream schedulers to
/// distinguish a stale-version abort from a real external conflict.
#[tokio::test]
async fn node_checkpoint_cas_conflict_surfaces_observed_status() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes"),
        EchoHandler,
    );

    let a = node_key!("a");
    let wf = make_workflow(
        vec![NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap()],
        vec![],
    );
    let stores = TestStores::new();
    stores.save_workflow(&wf).await;

    // Inject the external mutation before commit #1 — the first
    // call after `create()` is the node-level checkpoint
    // (v=0 → v=1 expected). The external bump flips status to
    // `cancelling` and moves the row to v=1 so the engine's
    // checkpoint_node CAS lands stale.
    let inner = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mutating = Arc::new(ExternalMutateBeforeN::new(
        inner.clone(),
        1,
        Some("cancelling"),
    ));
    let execution_stores = crate::store_seam::ExecutionStores {
        execution: mutating,
        journal: stores.journal.clone(),
        node_results: stores.node_results.clone(),
        checkpoints: stores.checkpoints.clone(),
        idempotency: stores.idempotency.clone(),
        resume_tokens: Arc::new(inner.resume_token_store()),
    };

    let (engine, _) = make_engine(registry);
    let engine = engine
        .with_execution_stores(execution_stores)
        .with_workflow_stores(stores.workflow_stores());

    // The final result is not the focus here — what matters is
    // that the persisted row shows the engine observed the
    // external status rather than blindly overwriting the row.
    // Note: depending on scheduling, the engine may report
    // Failed (node checkpoint aborted) or Cancelled (external).
    // Either way it MUST NOT claim Completed.
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;

    let execution_id_opt = match &result {
        Ok(r) => {
            assert_ne!(
                r.status,
                ExecutionStatus::Completed,
                "engine must not report Completed when the node's \
                     checkpoint CAS-missed against a concurrent external \
                     transition (§11.5, #333); got Completed for {r:?}"
            );
            Some(r.execution_id)
        },
        Err(e) => {
            // An execution-level Err is also acceptable — the
            // engine did not silently report success.
            tracing::debug!(error = %e, "execution surfaced typed error on CAS conflict");
            None
        },
    };

    // Crucially, the persisted row must still carry the external
    // `cancelling` status — never overwritten. The engine's own
    // writes after CAS miss MUST NOT land.
    if let Some(execution_id) = execution_id_opt {
        let scope = crate::store_seam::single_tenant_scope();
        let record = inner
            .get(&scope, &execution_id.to_string())
            .await
            .unwrap_or(None);
        if let Some(record) = record {
            let status = record.state.get("status").and_then(|v| v.as_str());
            assert_ne!(
                status,
                Some("completed"),
                "persisted row must not land as completed when the node \
                     checkpoint CAS-missed against a concurrent external \
                     mutation (§11.5, #333); found {status:?}"
            );
        }
    }
}

/// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
///
/// When the final CAS misses against a **non-terminal** external
/// write, the reconciliation helper retries once at the refreshed
/// version. The retry succeeds (no further concurrent writer), so
/// the engine commits its decision at the new version instead of
/// losing it. Pre-fix, the path was log-and-continue and the
/// engine's final write was silently dropped.
#[tokio::test]
async fn persist_final_state_retries_once_on_nonterminal_conflict() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);

    // Port-seam equivalent of the legacy #333 non-terminal-conflict
    // case: a fresh row has `fencing_generation == 0`, so the gen-0
    // token is the current holder and the engine can `commit`
    // without acquiring a lease. The external bump is itself a
    // gen-0 `commit` (the port analog of the old
    // `ExecutionRepo::transition`) — a pure CAS race, not a
    // fencing race. The reconciliation contract is identical:
    // non-terminal conflict ⇒ retry at the refreshed version
    // succeeds ⇒ `Ok(None)` and the engine's Completed decision is
    // durably persisted.
    let stores = TestStores::new();
    let execution = stores.execution.clone();
    let scope = crate::store_seam::single_tenant_scope();
    let token = nebula_storage_port::FencingToken::from_generation(0);

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let node_ids = vec![node_key!("x")];
    let mut local_state = ExecutionState::new(execution_id, workflow_id, &node_ids);
    local_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    execution
        .create(
            &scope,
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::to_value(&local_state).unwrap(),
        )
        .await
        .unwrap();

    // External non-terminal bump: stay in Running but advance
    // `updated_at`, committed at the baseline version v=0.
    let mut external_state = local_state.clone();
    external_state.updated_at = Utc::now();
    let external_json = serde_json::to_value(&external_state).unwrap();
    let external_outcome = execution
        .commit(
            nebula_storage_port::TransitionBatch::builder()
                .scope(scope.clone())
                .execution_id(execution_id.to_string())
                .expected_version(0)
                .fencing(token)
                .new_state(external_json)
                .build()
                .unwrap(),
        )
        .await
        .expect("external commit should succeed");
    assert!(
        matches!(
            external_outcome,
            nebula_storage_port::TransitionOutcome::Applied { new_version: 1 }
        ),
        "external commit must apply at v=1, got {external_outcome:?}"
    );

    // Engine's local final state is Completed, using the stale
    // repo_version=0.
    let mut repo_version: u64 = 0;
    let mut engine_final_state = local_state.clone();
    engine_final_state
        .transition_status(ExecutionStatus::Completed)
        .unwrap();

    let outcome = engine
        .persist_final_state_port(
            &scope,
            &stores.execution_stores(),
            execution_id,
            &mut engine_final_state,
            &mut repo_version,
            token,
        )
        .await
        .expect("retry should succeed on non-terminal conflict");

    assert_eq!(
        outcome, None,
        "helper must report Ok(None) when the local final status \
             was ultimately persisted (non-terminal conflict → retry \
             succeeded). got {outcome:?}"
    );

    // The persisted row must now be Completed at a bumped version.
    let (persisted_version, final_state) = stores
        .get_state(execution_id)
        .await
        .unwrap()
        .expect("row must still exist");
    assert!(
        persisted_version >= 2,
        "expected version ≥ 2 (create + external bump + retry), \
             got {persisted_version}"
    );
    assert_eq!(
        final_state.get("status").and_then(|v| v.as_str()),
        Some("completed"),
        "retry must durably persist the engine's Completed decision \
             at the refreshed version (§11.5, #333)"
    );
}

/// Regression for [#333](https://github.com/vanyastaff/nebula/issues/333).
///
/// Unit-level check on the reconciliation helper: when the final
/// CAS misses against a concurrent Cancelled write, the helper
/// returns `Ok(Some(Cancelled))` — not `Ok(None)` (silent overwrite
/// on the pre-fix path). Isolated from the full `execute_workflow`
/// frame so the observable contract is easy to evolve.
#[tokio::test]
async fn persist_final_state_honors_external_terminal_transition() {
    let registry = Arc::new(ActionRegistry::new());
    let (engine, _) = make_engine(registry);

    // Port-seam equivalent of the legacy #333 terminal-conflict
    // case. Same gen-0 reasoning as the non-terminal sibling: a
    // fresh row's fencing generation is 0, so the engine commits
    // with the gen-0 token and the external cancel is a gen-0
    // `commit`. The reconciliation contract is identical: an
    // external *terminal* write is honored — the helper returns
    // `Ok(Some(Cancelled))` and the engine's local Completed
    // decision must NOT overwrite the durable Cancelled row.
    let stores = TestStores::new();
    let execution = stores.execution.clone();
    let scope = crate::store_seam::single_tenant_scope();
    let token = nebula_storage_port::FencingToken::from_generation(0);

    let execution_id = ExecutionId::new();
    let workflow_id = WorkflowId::new();
    let node_ids = vec![node_key!("x")];
    let mut local_state = ExecutionState::new(execution_id, workflow_id, &node_ids);
    local_state
        .transition_status(ExecutionStatus::Running)
        .unwrap();
    execution
        .create(
            &scope,
            &execution_id.to_string(),
            &workflow_id.to_string(),
            serde_json::to_value(&local_state).unwrap(),
        )
        .await
        .unwrap();

    // Simulate an external cancel: commit status=cancelled at the
    // baseline version v=0.
    let mut external_state = local_state.clone();
    external_state
        .transition_status(ExecutionStatus::Cancelling)
        .ok();
    external_state
        .transition_status(ExecutionStatus::Cancelled)
        .ok();
    let external_json = serde_json::to_value(&external_state).unwrap();
    let external_outcome = execution
        .commit(
            nebula_storage_port::TransitionBatch::builder()
                .scope(scope.clone())
                .execution_id(execution_id.to_string())
                .expected_version(0)
                .fencing(token)
                .new_state(external_json)
                .build()
                .unwrap(),
        )
        .await
        .expect("external commit should succeed");
    assert!(
        matches!(
            external_outcome,
            nebula_storage_port::TransitionOutcome::Applied { new_version: 1 }
        ),
        "external cancel commit must apply at v=1, got {external_outcome:?}"
    );

    // Now ask the engine to persist its final state as Completed
    // starting from the stale version it had before the external
    // bump. This is exactly the pre-fix silent-overwrite scenario.
    let mut repo_version: u64 = 0;
    let mut engine_final_state = local_state.clone();
    engine_final_state
        .transition_status(ExecutionStatus::Completed)
        .unwrap();

    let outcome = engine
        .persist_final_state_port(
            &scope,
            &stores.execution_stores(),
            execution_id,
            &mut engine_final_state,
            &mut repo_version,
            token,
        )
        .await
        .expect("reconciliation must succeed against an external terminal write");

    assert_eq!(
        outcome,
        Some(ExecutionStatus::Cancelled),
        "helper must report the external terminal status; pre-fix \
             returned None and silently overwrote the cancel (§11.5, #333). \
             got {outcome:?}"
    );

    // Double-check: the persisted row still says `cancelled`.
    let (_v, final_state) = stores
        .get_state(execution_id)
        .await
        .unwrap()
        .expect("row must still exist");
    assert_eq!(
        final_state.get("status").and_then(|v| v.as_str()),
        Some("cancelled"),
        "engine must not overwrite the external Cancelled row \
             with its local Completed decision (§11.5, #333)"
    );
}

// ── #325 execution lease lifecycle (ADR 0008) ─────────────────────────

/// Regression for #325: a second `resume_execution` on the same row
/// while a first runner still holds the lease must get
/// [`EngineError::Leased`] instead of racing the frontier loop.
///
/// This is the core multi-runner correctness property. We construct
/// the repo by hand, seed a non-terminal execution row, and call
/// `resume_execution` twice concurrently. Only one runner may
/// dispatch nodes.
#[tokio::test]
async fn two_concurrent_resume_runners_are_fenced_by_lease() {
    let registry = Arc::new(ActionRegistry::new());
    // Slow echo so the first runner is still inside the frontier
    // loop when the second call arrives.
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("slow"), "Slow", "slow echoes"),
        SlowHandler {
            delay: Duration::from_millis(300),
        },
    );

    let stores = TestStores::new();
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "Slow", "core", "slow").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;

    // Seed a non-terminal execution row the two runners will target.
    let execution_id = ExecutionId::new();
    let node_ids = vec![n.clone()];
    let exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
    let state_json = serde_json::to_value(&exec_state).unwrap();
    stores.inject_state(execution_id, wf.id, state_json).await;

    // Two independent engines, each with its own InstanceId, sharing
    // the same storage. One of them should win the lease.
    let (engine_a, _) = make_engine(registry.clone());
    let engine_a = stores.attach(engine_a);
    let (engine_b, _) = make_engine(registry);
    let engine_b = stores.attach(engine_b);

    assert_ne!(
        engine_a.instance_id(),
        engine_b.instance_id(),
        "independent engines must produce distinct lease holder strings"
    );

    // Spawn both calls concurrently — whoever acquires first wins,
    // the other must see `EngineError::Leased`.
    let handle_a = tokio::spawn(async move {
        engine_a
            .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
            .await
    });
    let handle_b = tokio::spawn(async move {
        engine_b
            .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
            .await
    });

    let result_a = handle_a.await.unwrap();
    let result_b = handle_b.await.unwrap();

    let losses: Vec<_> = [&result_a, &result_b]
        .iter()
        .filter_map(|r| match r {
            Err(EngineError::Leased {
                execution_id: eid,
                holder,
            }) => Some((*eid, holder.clone())),
            _ => None,
        })
        .collect();
    let successes: Vec<_> = [&result_a, &result_b]
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    assert_eq!(
        losses.len() + successes.len(),
        2,
        "both calls must return either Ok or a typed Leased error, no panics; \
             got a={result_a:?}, b={result_b:?}"
    );
    assert_eq!(
        losses.len(),
        1,
        "exactly one runner must be fenced by the lease; got a={result_a:?}, b={result_b:?}"
    );
    assert_eq!(
        successes.len(),
        1,
        "exactly one runner must dispatch nodes; got a={result_a:?}, b={result_b:?}"
    );
    assert_eq!(
        losses[0].0, execution_id,
        "Leased error must carry the execution id that was contested"
    );
    assert!(
        successes[0].is_success(),
        "the winning runner must complete the workflow successfully; \
             got status={:?}",
        successes[0].status
    );
}

/// Registry race regression.
///
/// Two `resume_execution` calls overlap on the **same engine** for the
/// **same execution_id**. The winner acquires the lease and publishes
/// its token into `running`; the loser hits `EngineError::Leased`
/// before it ever inserts — so no drop guard can clobber the winner's
/// entry. This asserts the observable contract: while the winner's
/// frontier loop is live, `engine.cancel_execution(id)` still finds a
/// registered token even after the loser has returned.
#[tokio::test]
async fn overlapping_resume_losers_do_not_clobber_winners_registry_entry() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("slow"), "Slow", "slow echoes"),
        SlowHandler {
            delay: Duration::from_millis(500),
        },
    );

    let stores = TestStores::new();
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "Slow", "core", "slow").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;

    let execution_id = ExecutionId::new();
    let node_ids = vec![n.clone()];
    let exec_state = ExecutionState::new(execution_id, wf.id, &node_ids);
    stores
        .inject_state(
            execution_id,
            wf.id,
            serde_json::to_value(&exec_state).unwrap(),
        )
        .await;

    // Single engine, so both calls share the same `running` registry —
    // this is the path the Copilot review flagged. Wrap in `Arc` so we
    // can drive the second call from a background task and still
    // observe the registry from the test thread.
    let (engine, _) = make_engine(registry);
    let engine = Arc::new(stores.attach(engine));

    // Winner: drive the workflow in the background. Its frontier loop
    // will be live (500ms sleep) long enough for the loser to race.
    let winner_engine = Arc::clone(&engine);
    let winner = tokio::spawn(async move {
        winner_engine
            .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
            .await
    });

    // Poll the registry until the winner has published its token.
    // This synchronises on the exact moment the race window opens.
    let t_wait = Instant::now();
    loop {
        if engine.running.contains_key(&execution_id) {
            break;
        }
        assert!(
            t_wait.elapsed() < Duration::from_secs(2),
            "winner failed to register its token within 2s"
        );
        tokio::task::yield_now().await;
    }

    // Loser: a second resume call on the same engine for the same id.
    // Must fail fast with `Leased` — and crucially must NOT clobber
    // the registry entry the winner just published.
    let loser = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await;
    assert!(
        matches!(loser, Err(EngineError::Leased { .. })),
        "overlapping resume must be fenced by the lease; got {loser:?}"
    );

    // The winner is still running — its token must still be live.
    // This is the property that would have failed without the
    // vacant-only insert + nonce-scoped remove_if (Copilot hazard).
    assert!(
        engine.cancel_execution(execution_id),
        "winner's registry entry must survive the loser's failed attempt \
             (if this fails, the loser's Drop clobbered the winner's token)"
    );

    // Signalling cancel aborts the winner quickly. Assert the outcome
    // explicitly — `Err(EngineError::Leased)` or any non-terminal status
    // would indicate a real regression (e.g. the heartbeat unexpectedly
    // stole the lease during the 500ms slow handler); silently dropping
    // the `Result` would mask that.
    let winner_result = tokio::time::timeout(Duration::from_secs(5), winner)
        .await
        .expect("winner returns within 5s of cancel")
        .expect("join ok")
        .expect("winner returns Ok(ExecutionResult) after cancel");
    // Both labels are acceptable: the abort-select arm ends in `Cancelled`;
    // the node-error arm (handler returned `ActionError::Cancelled`, processed
    // as node failure) ends in `Failed`. The scheduler race between the two is
    // pre-existing behaviour covered by `integration::cancellation_via_sibling_failure`.
    assert!(
        matches!(
            winner_result.status,
            ExecutionStatus::Cancelled | ExecutionStatus::Failed
        ),
        "winner must reach a terminal non-success status after cancel; got {:?}",
        winner_result.status
    );
}

/// After the first runner releases the lease on terminal completion,
/// a later `resume_execution` on a non-terminal row can acquire it
/// cleanly. Covers the release-on-terminal branch of ADR 0008.
#[tokio::test]
async fn lease_is_released_after_terminal_completion_so_next_runner_can_acquire() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "echo", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;
    let engine = stores.attach(engine);

    // First run acquires + releases the lease on completion.
    let first = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("v1"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(first.is_success());

    // Lease must be free immediately — a brand-new acquire with a
    // fresh holder should succeed without waiting for TTL.
    let acquired = stores
        .acquire_lease(first.execution_id, "probe", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        acquired,
        "lease must be released on terminal completion, not pending TTL expiry"
    );
}

/// Once the engine has run an execution to completion, a second
/// `execute_workflow` call produces a brand-new `ExecutionId`, so
/// its lease is independent and acquires without contention.
/// Defense-in-depth: confirms we don't share lease state across
/// unrelated ids.
#[tokio::test]
async fn execute_workflow_produces_independent_lease_per_execution_id() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echoes input"),
        EchoHandler,
    );

    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "echo", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&wf).await;
    let engine = stores.attach(engine);

    let first = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("v1"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    let second = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!("v2"),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();
    assert!(first.is_success());
    assert!(second.is_success());
    assert_ne!(
        first.execution_id, second.execution_id,
        "each execute_workflow call must produce its own ExecutionId"
    );
}

// ── Factory dispatch regression guard ────────────────────────────────────
//
// These tests pin the contract that registrations via
// `register_*_factory::<A>()` flow through `factory.instantiate`
// at dispatch. The legacy spine was deleted in ADR-0098 D0 PR3;
// these fixtures guard against any regression that skips instantiation.

/// Variant A fixture — counts how many times `from_workflow_node` is
/// called so the test can assert the factory path was taken.
struct FactoryEcho;

impl Action for FactoryEcho {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.factory.echo"),
            "FactoryEcho",
            "echo via factory dispatch",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for FactoryEcho {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

/// Global instantiation counter — every call to
/// [`FactoryEcho::from_workflow_node`] bumps it. Tests serialize on
/// [`FACTORY_TEST_LOCK`] (an async `tokio::sync::Mutex`, so the
/// guard can be held across `.await` points without tripping
/// `clippy::await_holding_lock`).
static FACTORY_INSTANTIATIONS: AtomicU32 = AtomicU32::new(0);
static FACTORY_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl nebula_action::FromWorkflowNode for FactoryEcho {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &NodeDefinition,
        _ctx: &dyn nebula_action::ActionContext,
    ) -> Result<Self, Self::Error> {
        FACTORY_INSTANTIATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}

#[tokio::test]
async fn workflow_node_dispatches_through_factory_path() {
    let _guard = FACTORY_TEST_LOCK.lock().await;
    let baseline = FACTORY_INSTANTIATIONS.load(Ordering::SeqCst);

    let registry = Arc::new(ActionRegistry::new());
    // Production registration helper — wires `Arc<dyn ActionFactory>`,
    // not `ActionHandler`. The runtime's dispatch path MUST call
    // `factory.instantiate(node, ctx)` for each dispatch, which in
    // turn calls `FactoryEcho::from_workflow_node`, which bumps
    // FACTORY_INSTANTIATIONS.
    registry.register_stateless_factory::<FactoryEcho>();

    let (engine, _) = make_engine(registry);

    let n = node_key!("n");
    let wf = make_workflow(
        vec![NodeDefinition::new(n.clone(), "factory_echo", "core", "test.factory.echo").unwrap()],
        vec![],
    );

    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!({"hello": "factory"}),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow should succeed");

    assert!(result.is_success(), "workflow result should be Success");
    assert_eq!(
        result.node_output(&n),
        Some(&serde_json::json!({"hello": "factory"}))
    );

    let after = FACTORY_INSTANTIATIONS.load(Ordering::SeqCst);
    assert_eq!(
        after - baseline,
        1,
        "factory.instantiate should have been called exactly once for the single dispatch"
    );
}

// There is intentionally no factory-vs-legacy precedence test: the scenario
// (a factory competing with a legacy `ActionHandler` for the same key) is
// structurally impossible now that the engine has a single dispatch spine.
// The surviving guarantee — factory dispatch is the sole execution path — is
// covered by `workflow_node_dispatches_through_factory_path`.

// ── determine_final_status priority-4a unit tests ─────────────────────

// `determine_final_status` is a pure function — these unit tests construct
// the `ExecutionState` by direct field override, bypassing the FSM transition
// table. This is the same pattern the `final_status_*` tests above use for
// constructing terminal states. `park_node` requires `Running` as a
// precondition (the engine-level invariant), which we don't need to enforce
// in a pure-function test. We set `.state` and `.next_attempt_at` directly.

/// **Priority-4a: single `Waiting{next_attempt_at:None}` → `Paused`.**
///
/// The frontier exits with one signal-driven waiting node (no timer wake).
/// Priority-4a must fire before priority-4 and return `Paused`, not
/// `Failed+FrontierIntegrityViolation`.
///
/// **Falsifiability (red-on-revert)**: remove priority-4a (or rename it
/// to emit `FrontierIntegrityViolation`) → the old priority-4 fires →
/// `decision.status == Failed` → the `== Paused` assertion fails.
#[test]
fn final_status_signal_waiting_node_returns_paused() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let n1 = node_key!("n1");

    let mut exec_state = ExecutionState::new(exec_id, wf_id, std::slice::from_ref(&n1));
    // Direct override — `next_attempt_at` defaults to None, so only the
    // state field needs to be set to model a signal-driven Waiting node.
    exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Waiting;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Paused,
        "a single signal-driven Waiting node must yield Paused, not Failed or Completed"
    );
    assert!(
        decision.integrity_violation.is_none(),
        "priority-4a must not populate integrity_violation — that is only for real bugs"
    );
    assert!(
        decision.termination_reason.is_none(),
        "Paused awaiting signal carries no engine-attributed termination reason"
    );
}

/// **Priority-4a guard 1: all-terminal run must NOT become `Paused`.**
///
/// If every node is terminal the frontier reached natural completion.
/// Priority-4a requires `!non_terminal_signal_waits.is_empty()`, so an
/// all-terminal state must fall through to priority-5 `Completed`.
#[test]
fn final_status_all_terminal_does_not_become_paused() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let n1 = node_key!("n1");

    let mut exec_state = ExecutionState::new(exec_id, wf_id, std::slice::from_ref(&n1));
    // Manually set terminal state — bypass normal transition rules so we can
    // construct the all-terminal scenario for a pure function test.
    exec_state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Completed,
        "an all-terminal run must reach Completed, never Paused (guard 1)"
    );
}

/// **Priority-4a guard 2: `Waiting{None} + Running` falls through to integrity violation.**
///
/// One `Waiting{next_attempt_at:None}` node PLUS one `Running` node is a
/// mixed set that indicates a real frontier bug (the loop exited while a
/// node was still dispatched). Priority-4a must NOT fire — the integrity
/// violation arm (priority-4) must catch it.
///
/// **Falsifiability**: remove the `has_active_non_signal_node` guard →
/// priority-4a fires on the mixed set → `decision.status == Paused` →
/// `!= Failed` assertion fails.
#[test]
fn final_status_mixed_waiting_and_running_triggers_integrity_violation() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let waiting_node = node_key!("waiting");
    let running_node = node_key!("running");

    let mut exec_state = ExecutionState::new(
        exec_id,
        wf_id,
        &[waiting_node.clone(), running_node.clone()],
    );
    // Override both nodes — bypassing the FSM to construct the impossible-at-
    // runtime (frontier exited with a live Running node) bug scenario.
    exec_state.node_states.get_mut(&waiting_node).unwrap().state = NodeState::Waiting;
    exec_state.node_states.get_mut(&running_node).unwrap().state = NodeState::Running;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Failed,
        "a mixed Waiting+Running frontier must trigger the integrity violation (guard 2)"
    );
    assert!(
        decision.integrity_violation.is_some(),
        "integrity_violation must be populated for the mixed-frontier bug"
    );
}

/// **Priority-4a guard 2: `Waiting{None} + Pending` returns `Paused`.**
///
/// A `Pending` downstream node that hasn't been activated yet (because its
/// upstream dependency is parked) is NOT a frontier bug — it is the normal
/// blocked-downstream state. Priority-4a must fire and return `Paused`
/// even when `Pending` nodes are present alongside signal-driven `Waiting`
/// nodes.
///
/// **Falsifiability**: revert guard-2 to the old `len == count` check →
/// the `Pending` node inflates `non_terminal_count` → `1 != 2` → guard-2
/// fails → priority-4 fires → `decision.status == Failed` → `!= Paused`
/// assertion fails.
#[test]
fn final_status_waiting_plus_pending_returns_paused() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let webhook_node = node_key!("webhook");
    let downstream_node = node_key!("downstream");

    let mut exec_state =
        ExecutionState::new(exec_id, wf_id, &[webhook_node.clone(), downstream_node]);
    // Upstream is a signal-driven Waiting node; downstream stays Pending
    // (never activated because its upstream dependency is still waiting).
    exec_state.node_states.get_mut(&webhook_node).unwrap().state = NodeState::Waiting;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Paused,
        "Waiting{{None}} + Pending downstream must be Paused, not a frontier violation"
    );
    assert!(
        decision.integrity_violation.is_none(),
        "no integrity violation expected when the only non-wait nodes are Pending"
    );
}

/// **Priority-4a guard 2: `Waiting{None} + WaitingRetry` triggers integrity violation.**
///
/// A `WaitingRetry` node at frontier-exit is ANOMALOUS, not a benign park.
/// The frontier's exit condition requires `retry_heap.is_empty()`: a
/// `WaitingRetry` node present when the loop exits means its heap entry
/// was lost — a genuine frontier bug. Priority-4a MUST NOT mask this by
/// returning `Paused`; the integrity violation arm (priority-4) must catch it.
///
/// **Falsifiability**: narrow the `has_active_non_signal_node` check to
/// `Running`-only (drop `WaitingRetry`) → priority-4a fires on the mixed
/// set → `decision.status == Paused` → `!= Failed` assertion fails → RED.
#[test]
fn final_status_waiting_plus_waiting_retry_triggers_integrity_violation() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let signal_node = node_key!("signal");
    let retry_node = node_key!("retry");

    let mut exec_state =
        ExecutionState::new(exec_id, wf_id, &[signal_node.clone(), retry_node.clone()]);
    // Override both nodes — bypassing the FSM to construct the impossible-at-
    // runtime (retry_heap lost its entry) anomaly scenario.
    exec_state.node_states.get_mut(&signal_node).unwrap().state = NodeState::Waiting;
    exec_state.node_states.get_mut(&retry_node).unwrap().state = NodeState::WaitingRetry;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Failed,
        "Waiting{{None}} + WaitingRetry is a lost heap-entry anomaly — must be an \
             integrity violation, not masked as Paused"
    );
    assert!(
        decision.integrity_violation.is_some(),
        "integrity_violation must be populated: WaitingRetry at frontier-exit is a bug"
    );
}

/// **Priority-4a guard 2: `Waiting{None} + Ready` triggers integrity violation.**
///
/// A `Ready` node at frontier-exit means the node was activated and queued
/// for dispatch but the frontier exited without spawning it — runnable work
/// was stranded. (A node merely *blocked behind* a signal wait stays
/// `Pending`; it never reaches `Ready` because its wait predecessor is
/// non-terminal and so never activates it.) Priority-4a MUST NOT mask this
/// stranded-work bug as `Paused`.
///
/// **Falsifiability**: drop `NodeState::Ready` from `has_non_benign_non_terminal_node`
/// → priority-4a fires on the mixed set → `decision.status == Paused` →
/// `!= Failed` assertion fails → RED.
#[test]
fn final_status_waiting_plus_ready_triggers_integrity_violation() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let signal_node = node_key!("signal");
    let ready_node = node_key!("ready");

    let mut exec_state =
        ExecutionState::new(exec_id, wf_id, &[signal_node.clone(), ready_node.clone()]);
    // Override both nodes — bypassing the FSM to construct the impossible-at-
    // runtime (frontier exited leaving a Ready node unspawned) anomaly.
    exec_state.node_states.get_mut(&signal_node).unwrap().state = NodeState::Waiting;
    exec_state.node_states.get_mut(&ready_node).unwrap().state = NodeState::Ready;

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Failed,
        "Waiting{{None}} + Ready is stranded runnable work — must be an integrity \
             violation, not masked as Paused"
    );
    assert!(
        decision.integrity_violation.is_some(),
        "integrity_violation must be populated: a Ready node at frontier-exit is a bug"
    );
}

/// **Priority-4a guard 2: `Waiting{None} + timer Waiting{Some}` triggers integrity
/// violation.**
///
/// The frontier loop only exits once `wait_heap.is_empty()`, and Phase-0b
/// drains every due timer wait to `Completed` before that. A timer wait
/// (`next_attempt_at == Some`) still present at frontier-exit therefore means
/// its `wait_heap` entry was lost — the same lost-entry anomaly as
/// `WaitingRetry`. Priority-4a MUST NOT mask it as a benign signal `Paused`.
///
/// **Falsifiability**: drop the `Waiting && next_attempt_at.is_some()` clause
/// from `has_non_benign_non_terminal_node` → priority-4a fires → `decision.status
/// == Paused` → `!= Failed` assertion fails → RED.
#[test]
fn final_status_signal_wait_plus_timer_wait_triggers_integrity_violation() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let signal_node = node_key!("signal");
    let timer_node = node_key!("timer");

    let mut exec_state =
        ExecutionState::new(exec_id, wf_id, &[signal_node.clone(), timer_node.clone()]);
    // signal wait: Waiting{next_attempt_at: None}; timer wait: Waiting{Some}.
    // A timer wait left at frontier-exit is a lost wait_heap entry (the loop
    // only exits when wait_heap is empty).
    exec_state.node_states.get_mut(&signal_node).unwrap().state = NodeState::Waiting;
    {
        let timer_ns = exec_state.node_states.get_mut(&timer_node).unwrap();
        timer_ns.state = NodeState::Waiting;
        timer_ns.next_attempt_at = Some(Utc::now());
    }

    let cancel_token = CancellationToken::new();
    let decision = determine_final_status(&None, &cancel_token, &exec_state);

    assert_eq!(
        decision.status,
        ExecutionStatus::Failed,
        "a timer Waiting{{Some}} node at frontier-exit is a lost wait_heap entry — must be \
             an integrity violation, not masked as Paused by the co-present signal wait"
    );
    assert!(
        decision.integrity_violation.is_some(),
        "integrity_violation must be populated: a timer wait at frontier-exit is a bug"
    );
}

/// **Cancel teardown cancels signal waits that are not on `wait_heap`.**
///
/// Signal-driven waits (`next_attempt_at == None`) are intentionally not
/// heap-tracked, so the `wait_heap` drain in `drain_pending_to_cancelled`
/// never visits them. The added `node_states` scan must still transition
/// them `Waiting → Cancelled`, otherwise a cancel observed while a signal
/// wait is parked leaves the node non-terminal under a `Cancelled`
/// execution (a state-machine leak).
///
/// **Falsifiability**: remove the signal-wait scan from
/// `drain_pending_to_cancelled` → the node stays `Waiting` → the
/// `== Cancelled` assertion fails → RED.
#[test]
fn drain_pending_to_cancelled_cancels_signal_waits_not_on_heap() {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, VecDeque};

    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let signal_node = node_key!("signal");

    let mut exec_state = ExecutionState::new(exec_id, wf_id, std::slice::from_ref(&signal_node));
    // Signal wait: `Waiting{next_attempt_at: None}`, NOT on any heap.
    {
        let ns = exec_state.node_states.get_mut(&signal_node).unwrap();
        ns.state = NodeState::Waiting;
        ns.next_attempt_at = None;
    }

    // All heaps + ready_queue empty — the signal node is reachable only via
    // the node_states scan, not the heap drains.
    let mut retry_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>> = BinaryHeap::new();
    let mut wait_heap: BinaryHeap<Reverse<(DateTime<Utc>, NodeKey)>> = BinaryHeap::new();
    let mut ready_queue: VecDeque<NodeKey> = VecDeque::new();

    drain_pending_to_cancelled(
        &mut retry_heap,
        &mut wait_heap,
        &mut ready_queue,
        &mut exec_state,
        exec_id,
    );

    assert_eq!(
        exec_state.node_states.get(&signal_node).unwrap().state,
        NodeState::Cancelled,
        "a signal Waiting{{None}} node (not on wait_heap) must be cancelled by the \
             teardown scan"
    );
}

#[test]
fn mark_node_failed_does_not_stamp_error_on_a_non_failed_node() {
    use nebula_core::id::WorkflowId;
    use nebula_core::node_key;

    // A node already terminal in a NON-Failed state (Completed) must not
    // receive a failure error_message: the transition to Failed is rejected,
    // so the message write is skipped. Red-on-revert: the prior unconditional
    // write attached the failure string to the Completed node.
    let exec_id = ExecutionId::new();
    let wf_id = WorkflowId::new();
    let node = node_key!("done");

    let mut exec_state = ExecutionState::new(exec_id, wf_id, std::slice::from_ref(&node));
    exec_state.node_states.get_mut(&node).unwrap().state = NodeState::Completed;

    mark_node_failed(
        &mut exec_state,
        node.clone(),
        &EngineError::PlanningFailed("boom".into()),
    );

    let ns = exec_state.node_states.get(&node).unwrap();
    assert_eq!(
        ns.state,
        NodeState::Completed,
        "the node stays Completed — the Failed transition is rejected for a terminal node"
    );
    assert!(
        ns.error_message.is_none(),
        "no failure message is stamped onto a node that did not transition to Failed"
    );
}

// ── P1#1 resume_live channel mechanics (ADR-0099 W-S2b) ──────────────────
//
// These exercise `resume_live`'s request/reply channel directly, without a
// live frontier loop, so each not-durable outcome is fully deterministic.
// The loop-integration side (ack-after-checkpoint, fenced-out-arm,
// duplicate-resume) lives in `tests/wait_timeout.rs`.

/// Publish a `RunningEntry` for `execution_id` whose `resume_tx` is wired to
/// the returned receiver, so a test can simulate the loop side of the
/// channel. Returns the receiver and the registration guard (the entry is
/// removed when the guard drops).
fn publish_running_entry(
    engine: &WorkflowEngine,
    execution_id: ExecutionId,
) -> (mpsc::Receiver<ResumeRequest>, RunningRegistration) {
    let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
    let (resume_tx, resume_rx) = mpsc::channel::<ResumeRequest>(RESUME_CHANNEL_CAPACITY);
    engine.running.insert(
        execution_id,
        RunningEntry {
            registration_id,
            token: CancellationToken::new(),
            resume_tx,
        },
    );
    let guard = RunningRegistration {
        running: Arc::clone(&engine.running),
        execution_id,
        registration_id,
    };
    (resume_rx, guard)
}

/// `resume_live` returns `NoLiveEntry` when no `RunningEntry` exists for the
/// execution (the cross-runner / just-paused case). This is the preserved
/// no-live behavior the P1#1 channel rewrite must not regress.
#[tokio::test]
async fn resume_live_no_live_entry_when_absent() {
    let (engine, _) = make_engine(Arc::new(ActionRegistry::new()));
    let outcome = engine.resume_live(ExecutionId::new(), None).await;
    assert!(
        matches!(outcome, ResumeDelivery::NoLiveEntry),
        "an absent execution must yield NoLiveEntry, got {outcome:?}"
    );
}

/// `resume_live` returns `LoopGone` when the live loop receives the request
/// but drops the `ack` sender before replying (it exited / panicked). The
/// dropped sender resolves the awaiting receiver to `Err` — a free
/// fail-safe: a not-durable arm defers rather than acks.
#[tokio::test]
async fn resume_live_loop_gone_when_ack_dropped() {
    let (engine, _) = make_engine(Arc::new(ActionRegistry::new()));
    let engine = Arc::new(engine);
    let execution_id = ExecutionId::new();
    let (mut resume_rx, _guard) = publish_running_entry(&engine, execution_id);

    // Simulate a loop that takes the request then dies before replying.
    let loop_side = tokio::spawn(async move {
        let req = resume_rx.recv().await.expect("request must arrive");
        // Drop `req` (and its ack sender) without sending — models the loop
        // exiting mid-iteration.
        drop(req);
    });

    let outcome = engine.resume_live(execution_id, None).await;
    loop_side.await.unwrap();
    assert!(
        matches!(outcome, ResumeDelivery::LoopGone),
        "a dropped ack sender must yield LoopGone, got {outcome:?}"
    );
}

/// `resume_live` returns `AckTimeout` when the loop never replies within
/// `RESUME_ACK_TIMEOUT`. Driven under paused time so the test is instant and
/// deterministic. A wedged loop defers rather than acks.
#[tokio::test(start_paused = true)]
async fn resume_live_ack_timeout_when_loop_silent() {
    let (engine, _) = make_engine(Arc::new(ActionRegistry::new()));
    let engine = Arc::new(engine);
    let execution_id = ExecutionId::new();
    // Keep the receiver alive but NEVER reply, so the request is delivered
    // (channel open) yet the ack never resolves.
    let (_resume_rx, _guard) = publish_running_entry(&engine, execution_id);

    let engine_h = Arc::clone(&engine);
    let resume = tokio::spawn(async move { engine_h.resume_live(execution_id, None).await });
    // Advance past the ack timeout under paused time.
    tokio::time::advance(RESUME_ACK_TIMEOUT + Duration::from_secs(1)).await;
    let outcome = resume.await.unwrap();
    assert!(
        matches!(outcome, ResumeDelivery::AckTimeout),
        "a silent loop must yield AckTimeout, got {outcome:?}"
    );
}

// ── FIX 1: injectable Clock — retry deadline is derived from injected time ──
//
// `next_retry_at` previously called `Utc::now()` internally, making retry
// deadlines non-deterministic and impossible to assert in tests. After the
// fix it accepts `now: DateTime<Utc>` from the caller (who reads it from
// an injectable `Clock`). This test verifies that the returned deadline
// equals `now + delay` and is completely independent of wall time.

#[test]
fn next_retry_at_computes_deadline_from_injected_now() {
    use chrono::TimeZone;

    // Epoch-pinned fixed instant — wall time is irrelevant.
    let pinned_now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let delay = std::time::Duration::from_secs(30);

    let execution_id = ExecutionId::new();
    let node = nebula_core::node_key!("retry-test");
    let deadline = next_retry_at(execution_id, &node, delay, pinned_now);

    let expected = pinned_now + chrono::Duration::seconds(30);
    assert_eq!(
        deadline, expected,
        "retry deadline must be exactly pinned_now + delay, not derived from wall time"
    );
}

#[test]
fn next_retry_at_clamps_overflow_delay() {
    use chrono::TimeZone;

    // A delay that cannot be represented by chrono::Duration (> ~292 years).
    // next_retry_at must clamp to DateTime::<Utc>::MAX_UTC rather than panicking.
    // (A naive `now + chrono::Duration::MAX` overflows for any `now` > epoch.)
    let pinned_now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let overflow_delay = std::time::Duration::from_secs(u64::MAX / 2);

    let execution_id = ExecutionId::new();
    let node = nebula_core::node_key!("retry-overflow");
    let deadline = next_retry_at(execution_id, &node, overflow_delay, pinned_now);

    assert_eq!(
        deadline,
        DateTime::<Utc>::MAX_UTC,
        "overflow delay must clamp to MAX_UTC, not panic or return an arbitrary future time"
    );
}

// ── FIX 1: with_clock builder wires injected clock into engine ───────────
//
// Verifies that `with_clock` stores the injected clock and that the engine
// actually reads from it. Uses `clock_now()` (a `#[cfg(test)]` accessor on
// `WorkflowEngine`) to call through to the stored clock without needing to
// drive a full workflow execution.
//
// A reversion to hardcoded `Utc::now()` inside the engine would cause
// `clock_now()` to read `SystemClock::now()` instead of the pinned fake,
// but the pinned-constant tests on `next_retry_at` cover the free-function
// path. This test guards the builder-wiring layer specifically.

#[test]
fn with_clock_builder_wires_injected_clock_into_engine() {
    use std::time::Instant;

    use chrono::TimeZone;
    use nebula_core::accessor::Clock;

    /// A clock pinned to a fixed instant so `clock_now()` returns exactly
    /// that instant — not wall time — if (and only if) the engine reads from
    /// the injected clock rather than calling `Utc::now()` directly.
    struct PinnedClock {
        pinned: DateTime<Utc>,
    }

    impl Clock for PinnedClock {
        fn now(&self) -> DateTime<Utc> {
            self.pinned
        }
        fn monotonic(&self) -> Instant {
            Instant::now()
        }
    }

    let pinned_instant = Utc.with_ymd_and_hms(2020, 6, 15, 12, 0, 0).unwrap();
    let (engine, _metrics) = make_engine(Arc::new(ActionRegistry::new()));
    let engine = engine.with_clock(Arc::new(PinnedClock {
        pinned: pinned_instant,
    }));

    // The engine must consult the injected clock, not real wall time.
    // If the clock field is not wired, clock_now() would return SystemClock::now()
    // and this assertion would fail (wall time != 2020-06-15T12:00:00Z).
    assert_eq!(
        engine.clock_now(),
        pinned_instant,
        "engine must read from the injected PinnedClock, not SystemClock"
    );
}
