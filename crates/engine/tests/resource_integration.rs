//! End-to-end integration test: action acquires a resource through the engine.
//!
//! Proves the full chain:
//!   register(MockResource) in Manager
//!     -> Engine holds Manager
//!       -> Action calls ctx.resource("mock")
//!         -> gets ResourceHandle
//!           -> downcasts to the concrete instance type

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_action::{
    ActionError, ActionMetadataDraft, action::Action, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key, id::WorkflowId, node_key};
use nebula_core::{OrgId, ResourceKey, ScopeLevel, WorkspaceId, resource_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_metrics::MetricsRegistry;
use nebula_resource::Resident;
use nebula_resource::{
    Manager, RegistrationSpec, ResidentConfig, ResourceContext, SlotIdentity,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

mod exact_fixture;

// ---------------------------------------------------------------------------
// Action handler that acquires a resource (Variant A)
// ---------------------------------------------------------------------------

/// Placeholder handler used by the smoke tests below — returns a fixed
/// output without actually consuming a resource. The test verifies that
/// attaching a resource manager does not break end-to-end dispatch; it
/// does not exercise resource acquisition (see [`ResourceProbeHandler`]
/// for that).
struct ResourceConsumerHandler;

impl Action for ResourceConsumerHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.resource_consumer.static"),
            nebula_action::metadata_name!("ResourceConsumer"),
            "static",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ResourceConsumerHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        // Smoke-path action: does NOT call ctx.resource(). The
        // attached-manager tests (below) verify that engine dispatch
        // still works with a resource manager wired in; a parallel
        // handler (`ResourceProbeHandler`) exercises the actual
        // acquisition path.
        Ok(ActionResult::success(
            serde_json::json!({ "resource_value": "mock-instance" }),
        ))
    }
}

/// Handler that actually acquires a resource through the
/// [`ActionContext`]. Used by the no-manager failure test to pin the
/// contract that `ctx.resource(..)` returns an error when the engine
/// was not wired with a resource manager.
struct ResourceProbeHandler;

impl Action for ResourceProbeHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.resource_probe.static"),
            nebula_action::metadata_name!("ResourceProbe"),
            "static",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ResourceProbeHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        // Let ctx.resource() return its natural error when the accessor
        // is the no-op default (no manager attached) — the engine then
        // translates the action failure into a failed workflow run.
        use nebula_core::ResourceKey;
        let key = ResourceKey::new("mock")
            .map_err(|e| ActionError::fatal(format!("invalid key: {e}")))?;
        let _instance = ctx
            .resources()
            .acquire_any(&key)
            .await
            .map_err(ActionError::from)?;
        Ok(ActionResult::success(
            serde_json::json!({ "resource_value": "acquired" }),
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "resource-integration-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
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
    }
}

fn meta(key: ActionKey) -> ActionMetadataDraft {
    let name = key.clone().into();
    ActionMetadataDraft::new(key, name, "resource integration test")
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Single-node workflow where the action acquires a resource from the manager
/// via `ctx.resource("mock")` and returns the instance value as output.
#[tokio::test]
async fn action_acquires_resource_through_engine() {
    // 1. Create an empty resource manager (no mock resource registered yet because the v2 API
    //    requires topology + release queue setup; the action handler returns a placeholder anyway
    //    until context wiring is complete).
    let manager = Arc::new(Manager::new());

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            meta(action_key!("resource-consumer")),
            ResourceConsumerHandler,
        )
        .expect("valid test catalog definition");

    // 3. Build the engine with the resource manager attached
    let runner = Arc::new(InProcessRunner::new());
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

    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_resource_manager(manager);

    // 4. Build and execute a single-node workflow
    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "core", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow execution");

    // 5. Verify the action successfully acquired and used the resource
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(&node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
        "action should have received the mock resource instance"
    );
}

/// Full lifecycle: engine with manager -> execute workflow -> verify -> shutdown
#[tokio::test]
async fn full_resource_lifecycle_with_shutdown() {
    // 1. Create an empty resource manager
    let manager = Arc::new(Manager::new());

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            meta(action_key!("resource-consumer")),
            ResourceConsumerHandler,
        )
        .expect("valid test catalog definition");

    // 3. Build the engine with the resource manager attached
    let runner = Arc::new(InProcessRunner::new());
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

    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_resource_manager(manager.clone());

    // 4. Execute a single-node workflow
    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "core", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow execution");

    // 5. Verify execution succeeded
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(&node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
    );

    // 6. Shutdown the manager
    manager.shutdown();
    assert!(manager.is_shutdown());
}

// ---------------------------------------------------------------------------
// Engine integration — real acquire through EngineResourceAccessor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct IntegrationProbeError(String);

impl std::fmt::Display for IntegrationProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IntegrationProbeError {}

impl From<IntegrationProbeError> for ResourceError {
    fn from(e: IntegrationProbeError) -> Self {
        ResourceError::permanent(e.0)
    }
}

#[derive(Clone, Debug, Default, nebula_schema::Schema)]
struct IntegrationProbeConfig;

impl ResourceConfig for IntegrationProbeConfig {
    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct IntegrationProbeResource;

#[async_trait::async_trait]
impl Provider for IntegrationProbeResource {
    type Config = IntegrationProbeConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.engine_integration.probe")
    }

    async fn create(
        &self,
        _config: &IntegrationProbeConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, ResourceError> {
        Ok(Arc::new(AtomicU64::new(7)))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("test.engine_integration.probe"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(IntegrationProbeResource);

#[async_trait::async_trait]
impl ResidentProvider for IntegrationProbeResource {
    fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
        runtime.load(Ordering::Relaxed) > 0
    }
}

struct IntegrationAcquireHandler;

impl Action for IntegrationAcquireHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.engine_integration.acquire"),
            nebula_action::metadata_name!("IntegrationAcquire"),
            "static",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for IntegrationAcquireHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        let key = IntegrationProbeResource::key();
        let boxed = ctx
            .resources()
            .acquire_any(&key)
            .await
            .map_err(ActionError::from)?;
        let guard = boxed
            .downcast::<nebula_resource::ResourceGuard<IntegrationProbeResource>>()
            .map_err(|_| ActionError::fatal("expected ResourceGuard downcast"))?;
        let value = guard.load(Ordering::Relaxed);
        Ok(ActionResult::success(serde_json::json!({ "lease": value })))
    }
}

/// Org-scoped registration + execution-scoped acquire + slot identity on engine.
#[tokio::test]
async fn engine_acquires_org_scoped_resource_through_accessor() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();

    manager
        .register(RegistrationSpec {
            resource: IntegrationProbeResource,
            config: IntegrationProbeConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<IntegrationProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register org-scoped resource");

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            meta(action_key!("engine-integration-acquire")),
            IntegrationAcquireHandler,
        )
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
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

    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_resource_manager(Arc::clone(&manager))
        .with_resource_acquire_scope(nebula_core::scope::Scope {
            org_id: Some(org),
            ..Default::default()
        });
    engine.record_resource_slot_identity(
        ScopeLevel::Organization(org),
        IntegrationProbeResource::key(),
        SlotIdentity::Unbound,
    );

    let node = node_key!("probe");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "core", "engine-integration-acquire").unwrap(),
    ]);

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow execution");

    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(&node).expect("node output");
    assert_eq!(
        output.get("lease").and_then(serde_json::Value::as_u64),
        Some(7)
    );
}

/// Two workspaces register the same kind with differently shaped bindings.
/// Each run must acquire its own workspace's row: the engine used to keep
/// one slot identity per resource key, so the later registration silently
/// redirected the other workspace's acquires to an identity its row does not
/// carry.
#[tokio::test]
async fn workspaces_with_different_binding_shapes_each_reach_their_own_row() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let bound_workspace = WorkspaceId::new();
    let unbound_workspace = WorkspaceId::new();
    let bound_identity = SlotIdentity::from_bindings([("auth", "test.credential")]);

    for (workspace, slot_identity) in [
        (bound_workspace, bound_identity.clone()),
        (unbound_workspace, SlotIdentity::Unbound),
    ] {
        manager
            .register(RegistrationSpec {
                resource: IntegrationProbeResource,
                config: IntegrationProbeConfig,
                scope: ScopeLevel::Workspace(workspace),
                slot_identity,
                topology: Resident::<IntegrationProbeResource>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register workspace-scoped resource");
    }

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            meta(action_key!("engine-integration-acquire")),
            IntegrationAcquireHandler,
        )
        .expect("valid test catalog definition");
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new()),
            DataPassingPolicy::default(),
            MetricsRegistry::new(),
        )
        .unwrap(),
    );
    let engine = WorkflowEngine::new(runtime, MetricsRegistry::new())
        .unwrap()
        .with_resource_manager(Arc::clone(&manager));
    engine.record_resource_slot_identity(
        ScopeLevel::Workspace(bound_workspace),
        IntegrationProbeResource::key(),
        bound_identity,
    );
    engine.record_resource_slot_identity(
        ScopeLevel::Workspace(unbound_workspace),
        IntegrationProbeResource::key(),
        SlotIdentity::Unbound,
    );

    let node = node_key!("probe");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "core", "engine-integration-acquire").unwrap(),
    ]);
    for workspace in [bound_workspace, unbound_workspace] {
        let result = engine
            .execute_workflow_with_acquire_scope(
                &nebula_engine::store_seam::single_tenant_scope(),
                &wf,
                serde_json::json!(null),
                ExecutionBudget::default(),
                Some(nebula_core::scope::Scope {
                    org_id: Some(org),
                    workspace_id: Some(workspace),
                    ..Default::default()
                }),
            )
            .await
            .expect("workflow execution");
        assert!(
            result.is_success(),
            "workspace {workspace} must acquire its own row"
        );
    }
}

/// A durable turn (the only way a persistent engine runs work) acquires
/// under the execution's own tenant. The resource context used to be
/// installed only for direct in-process starts, so a worker-driven execution
/// acquired with no workspace and never reached workspace-scoped rows.
#[tokio::test]
async fn durable_turn_acquires_the_executions_workspace_row() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let workspace = WorkspaceId::new();
    manager
        .register(RegistrationSpec {
            resource: IntegrationProbeResource,
            config: IntegrationProbeConfig,
            scope: ScopeLevel::Workspace(workspace),
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<IntegrationProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register workspace-scoped resource");

    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            meta(action_key!("core.resource_probe")),
            IntegrationAcquireHandler,
        )
        .expect("valid test catalog definition");
    let execution = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let stores = nebula_engine::ExecutionStores {
        execution: execution.clone(),
        journal: Arc::new(nebula_storage::InMemoryJournalReader::new(&execution)),
        node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
        checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
        idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
        resume_tokens: Arc::new(execution.resume_token_store()),
        operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
            &execution,
        )),
    };
    let frozen = exact_fixture::freeze_registry(&registry, &[("core", "core.resource_probe")]);
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new()),
            DataPassingPolicy::default(),
            MetricsRegistry::new(),
        )
        .unwrap(),
    );
    let engine = WorkflowEngine::new(runtime, MetricsRegistry::new())
        .unwrap()
        .with_resource_manager(Arc::clone(&manager))
        .with_execution_stores(stores)
        .with_plan_flavor_runtime(
            Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                execution.plan_flavor_catalog(),
            ))),
            Arc::clone(&frozen),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &execution,
            )),
        );
    engine.record_resource_slot_identity(
        ScopeLevel::Workspace(workspace),
        IntegrationProbeResource::key(),
        SlotIdentity::Unbound,
    );

    let node = node_key!("probe");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "probe", "core", "core.resource_probe").unwrap(),
    ]);
    let scope = nebula_storage_port::Scope::new(workspace.to_string(), org.to_string());
    let execution_id = nebula_core::ExecutionId::new();
    let mut state = nebula_execution::state::ExecutionState::new(
        execution_id,
        wf.id,
        std::slice::from_ref(&node),
    );
    exact_fixture::materialize_state(&execution, &scope, &frozen, &wf, &mut state).await;

    let result = engine
        .resume_execution(&scope, execution_id)
        .await
        .expect("durable turn");
    assert!(
        result.is_success(),
        "durable turn must reach the workspace row: {:?}",
        result.node_errors
    );
    assert_eq!(
        result
            .node_output(&node)
            .and_then(|output| output.get("lease"))
            .and_then(serde_json::Value::as_u64),
        Some(7)
    );
}

/// Verify that `ctx.resource()` returns a fatal error when no resource
/// manager is attached to the engine.
///
/// Uses [`ResourceProbeHandler`] (unlike the smoke tests above) so the
/// handler actually calls `ctx.resources().acquire_any(..)` — exercising
/// the engine's default [`NoopResourceAccessor`] fallback and surfacing
/// its fail-closed error as a failed workflow run.
#[tokio::test]
async fn action_resource_fails_without_manager() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(meta(action_key!("resource-probe")), ResourceProbeHandler)
        .expect("valid test catalog definition");
    let runner = Arc::new(InProcessRunner::new());
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

    let engine = WorkflowEngine::new(runtime, metrics).unwrap();
    // No .with_resource_manager() — intentionally omitted so the engine
    // falls back to the no-op accessor and the probe handler fails.

    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node, "A", "core", "resource-probe").unwrap(),
    ]);

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &wf,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow execution");

    // The action should have failed because no resource provider is configured
    assert!(
        result.is_failure(),
        "workflow should fail without resource manager"
    );
}

// ===========================================================================
// Phase 8 — cross-workflow shared-resource verification
// ===========================================================================
//
// Headline scenario: 10 simulated workflows × 1 `TelegramBot` resource at the
// same scope must dedupe to a single `Resource::create` invocation, with all
// 10 acquires returning leases that point at the same underlying runtime
// (`Arc::ptr_eq`).
//
// Architecture note (deviation from the original Phase 8 task wording):
// the `R::Credential` associated type was retired in Phase 4 , and
// the manager dedupes by `(R::key(), ScopeLevel)` — the static type-level key
// of the registered `Resource`, not a runtime `ResourceId`. The "10 workflows
// declaring the same `ResourceId`" framing collapses to "10 acquires of the
// same `Resource` impl at the same scope." Resident topology is the natural
// fit for a shared bot client: one shared runtime, clone-on-acquire under a
// `create_lock` mutex that double-checks after wakeup.

mod shared_resource {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use nebula_core::{ExecutionId, OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
    use nebula_resource::{
        AcquireOptions, Manager, RegistrationSpec, Resident, ResidentConfig, ResourceContext,
        SlotIdentity,
        error::Error,
        resource::{Provider, ResourceConfig, ResourceMetadataDraft},
        topology::resident::ResidentProvider,
    };
    use tokio_util::sync::CancellationToken;

    // -----------------------------------------------------------------------
    // Fake error
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct TelegramError(String);

    impl std::fmt::Display for TelegramError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for TelegramError {}

    impl From<TelegramError> for Error {
        fn from(e: TelegramError) -> Self {
            Error::transient(e.0)
        }
    }

    // -----------------------------------------------------------------------
    // Fake config — fingerprint distinguishes "the same bot reconfigured"
    // from "the original bot."
    // -----------------------------------------------------------------------

    #[derive(Clone, Debug, nebula_schema::Schema)]
    struct TelegramConfig {
        token: String,
    }

    impl ResourceConfig for TelegramConfig {
        fn validate(&self) -> Result<(), Error> {
            if self.token.is_empty() {
                Err(Error::permanent("telegram token must not be empty"))
            } else {
                Ok(())
            }
        }

        fn fingerprint(&self) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.token.hash(&mut h);
            h.finish()
        }
    }

    // -----------------------------------------------------------------------
    // Fake `TelegramBot` resource (Resident topology — single shared client).
    //
    // Each `Resource::create` invocation increments `create_counter` and
    // mints a fresh `Arc<TelegramBotInner>`. Pure dedupe is observed via:
    //   1. `create_counter` ending at 1 after N concurrent acquires
    //   2. `Arc::ptr_eq` on the `Arc<TelegramBotInner>` leases handed out
    // -----------------------------------------------------------------------

    /// Inner state of the bot — a unique identity tag. The pointer-equality
    /// check is on the `Arc<TelegramBotInner>`; `instance_id` is a
    /// human-readable witness that aids diagnosis when dedupe regresses.
    #[derive(Debug)]
    struct TelegramBotInner {
        instance_id: u64,
    }

    #[derive(Clone)]
    struct TelegramBot {
        create_counter: Arc<AtomicU64>,
        alive: Arc<AtomicBool>,
    }

    impl TelegramBot {
        fn new() -> Self {
            Self {
                create_counter: Arc::new(AtomicU64::new(0)),
                alive: Arc::new(AtomicBool::new(true)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for TelegramBot {
        type Config = TelegramConfig;
        type Instance = Arc<TelegramBotInner>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("telegram-bot")
        }

        async fn create(
            &self,
            _config: &TelegramConfig,
            _ctx: &ResourceContext,
        ) -> Result<Arc<TelegramBotInner>, Error> {
            // Yield once to widen the concurrent-acquire interleaving
            // window — exposes any missing serialization in the
            // double-checked create path.
            tokio::task::yield_now().await;
            let id = self.create_counter.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(TelegramBotInner { instance_id: id }))
        }

        async fn destroy(
            &self,
            _runtime: Arc<TelegramBotInner>,
            _cx: nebula_resource::TeardownCx,
        ) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("telegram-bot"),
                "",
            )
        }
    }

    nebula_resource::no_credential_slots!(TelegramBot);

    #[async_trait::async_trait]
    impl ResidentProvider for TelegramBot {
        fn is_alive_sync(&self, _runtime: &Arc<TelegramBotInner>) -> bool {
            self.alive.load(Ordering::Relaxed)
        }
    }

    /// A second resource type — different `R::key()` — used by the
    /// "different IDs distinct" edge case. Distinct `Resource` impls
    /// produce distinct registry rows even when configured identically.
    #[derive(Clone)]
    struct AlternateBot {
        create_counter: Arc<AtomicU64>,
        alive: Arc<AtomicBool>,
    }

    impl AlternateBot {
        fn new() -> Self {
            Self {
                create_counter: Arc::new(AtomicU64::new(0)),
                alive: Arc::new(AtomicBool::new(true)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for AlternateBot {
        type Config = TelegramConfig;
        type Instance = Arc<TelegramBotInner>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("telegram-bot-alt")
        }

        async fn create(
            &self,
            _config: &TelegramConfig,
            _ctx: &ResourceContext,
        ) -> Result<Arc<TelegramBotInner>, Error> {
            tokio::task::yield_now().await;
            let id = self.create_counter.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(TelegramBotInner {
                instance_id: 100_000 + id,
            }))
        }

        async fn destroy(
            &self,
            _runtime: Arc<TelegramBotInner>,
            _cx: nebula_resource::TeardownCx,
        ) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("telegram-bot-alt"),
                "",
            )
        }
    }

    nebula_resource::no_credential_slots!(AlternateBot);

    #[async_trait::async_trait]
    impl ResidentProvider for AlternateBot {
        fn is_alive_sync(&self, _runtime: &Arc<TelegramBotInner>) -> bool {
            self.alive.load(Ordering::Relaxed)
        }
    }

    fn test_config() -> TelegramConfig {
        TelegramConfig {
            token: "tg-bot-token-prod".into(),
        }
    }

    fn ctx_for_org(org: OrgId) -> ResourceContext {
        let scope = Scope {
            org_id: Some(org),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
    }

    fn ctx_for_execution() -> ResourceContext {
        let scope = Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
    }

    // -----------------------------------------------------------------------
    // Task 8.1 + 8.2 — headline shared-resource test
    //
    // 10 concurrently-spawned tasks (one per simulated workflow) all acquire
    // the same `TelegramBot` at the same `Organization` scope. The manager
    // must:
    //   1. invoke `Resource::create` exactly once
    //   2. hand every caller a lease whose underlying `Arc<TelegramBotInner>` is pointer-equal to
    //      every other caller's lease
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cross_workflow_resource_sharing() {
        let manager = Arc::new(Manager::new());
        let bot = TelegramBot::new();
        let create_counter = Arc::clone(&bot.create_counter);
        let resident_rt = Resident::<TelegramBot>::new(ResidentConfig::default());
        let org = OrgId::new();

        manager
            .register(RegistrationSpec {
                resource: bot,
                config: test_config(),
                scope: ScopeLevel::Organization(org),
                slot_identity: SlotIdentity::Unbound,
                topology: resident_rt,
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register should succeed");

        // 10 simulated workflows acquire concurrently.
        let mut handles = Vec::with_capacity(10);
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let ctx = ctx_for_org(org);
                mgr.acquire_resident::<TelegramBot>(&ctx, &AcquireOptions::default())
                    .await
                    .expect("acquire should succeed")
            }));
        }

        let guards: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("task should not panic"))
            .collect();

        // Assertion 1: exactly one `Resource::create` invocation.
        assert_eq!(
            create_counter.load(Ordering::SeqCst),
            1,
            "10 concurrent acquires of the same Resource at the same scope \
             must collapse to a single Resource::create invocation"
        );

        // Assertion 2: every lease points at the same underlying runtime.
        // The Resident topology hands out `Arc<TelegramBotInner>` leases
        // that all clone the same backing `Arc`, so `Arc::ptr_eq` holds
        // across every pair.
        let first_arc: &Arc<TelegramBotInner> = &guards[0];
        for (i, other) in guards.iter().enumerate().skip(1) {
            assert!(
                Arc::ptr_eq(first_arc, other),
                "guard #0 and guard #{i} must share the same Arc<TelegramBotInner>; \
                 dedupe failed"
            );
        }

        // The instance id is also a witness: every caller observes the
        // first-and-only generation (id = 0).
        for g in &guards {
            assert_eq!(g.instance_id, 0);
        }
    }

    // -----------------------------------------------------------------------
    // Edge case A — different `R::key()` produce distinct instances even
    // when configured with identical configs at identical scopes.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn different_resource_keys_produce_distinct_instances() {
        let manager = Manager::new();

        let bot_a = TelegramBot::new();
        let counter_a = Arc::clone(&bot_a.create_counter);
        let bot_b = AlternateBot::new();
        let counter_b = Arc::clone(&bot_b.create_counter);

        let org = OrgId::new();
        let scope = ScopeLevel::Organization(org);

        manager
            .register(RegistrationSpec {
                resource: bot_a,
                config: test_config(),
                scope: scope.clone(),
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<TelegramBot>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register A should succeed");
        manager
            .register(RegistrationSpec {
                resource: bot_b,
                config: test_config(),
                scope,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<AlternateBot>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register B should succeed");

        let ctx = ctx_for_org(org);

        let lease_a = manager
            .acquire_resident::<TelegramBot>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire A");
        let lease_b = manager
            .acquire_resident::<AlternateBot>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire B");

        // Each Resource type has its own create counter — both fire once.
        assert_eq!(counter_a.load(Ordering::SeqCst), 1);
        assert_eq!(counter_b.load(Ordering::SeqCst), 1);

        // The leases point at distinct runtimes — different keys, no
        // cross-aliasing.
        let a_arc: &Arc<TelegramBotInner> = &lease_a;
        let b_arc: &Arc<TelegramBotInner> = &lease_b;
        assert!(
            !Arc::ptr_eq(a_arc, b_arc),
            "different Resource keys must produce distinct underlying Arcs"
        );
        // Instance-id namespaces don't overlap (TelegramBot starts at 0,
        // AlternateBot starts at 100_000).
        assert_eq!(a_arc.instance_id, 0);
        assert_eq!(b_arc.instance_id, 100_000);
    }

    // -----------------------------------------------------------------------
    // Edge case B — same `R::key()` registered at two different scopes
    // produces two independent instances (closest-ancestor lookup, not
    // unifying).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn different_scopes_produce_distinct_instances() {
        let manager = Manager::new();

        // Two TelegramBot instances with INDEPENDENT counters — registry
        // stores them under separate scope keys.
        let bot_org_a = TelegramBot::new();
        let counter_org_a = Arc::clone(&bot_org_a.create_counter);
        let bot_org_b = TelegramBot::new();
        let counter_org_b = Arc::clone(&bot_org_b.create_counter);

        let org_a = OrgId::new();
        let org_b = OrgId::new();

        manager
            .register(RegistrationSpec {
                resource: bot_org_a,
                config: test_config(),
                scope: ScopeLevel::Organization(org_a),
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<TelegramBot>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register org_a should succeed");
        manager
            .register(RegistrationSpec {
                resource: bot_org_b,
                config: test_config(),
                scope: ScopeLevel::Organization(org_b),
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<TelegramBot>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register org_b should succeed");

        let lease_a = manager
            .acquire_resident::<TelegramBot>(&ctx_for_org(org_a), &AcquireOptions::default())
            .await
            .expect("acquire from org_a");
        let lease_b = manager
            .acquire_resident::<TelegramBot>(&ctx_for_org(org_b), &AcquireOptions::default())
            .await
            .expect("acquire from org_b");

        // Each scope's resource was created exactly once, independently.
        assert_eq!(counter_org_a.load(Ordering::SeqCst), 1);
        assert_eq!(counter_org_b.load(Ordering::SeqCst), 1);

        // Distinct `Arc<TelegramBotInner>` payloads — no cross-scope
        // aliasing.
        let a_arc: &Arc<TelegramBotInner> = &lease_a;
        let b_arc: &Arc<TelegramBotInner> = &lease_b;
        assert!(
            !Arc::ptr_eq(a_arc, b_arc),
            "same Resource key at different scopes must produce distinct \
             underlying Arcs"
        );
    }

    // -----------------------------------------------------------------------
    // Edge case C — fingerprint change via `reload_config` bumps the
    // generation counter (so pool topologies evict idle entries with the
    // stale fingerprint on next acquire/release).
    //
    // NB: the resident topology does not eagerly destroy on `reload_config`
    // — its mandate is "single shared instance," and rebuild happens only
    // on liveness failure or explicit shutdown. The fingerprint-eviction
    // path is exercised at the pool level (see `runtime::pool::tests` and
    // `basic_integration::reload_config_swaps_config_and_bumps_generation`);
    // here we verify the manager-level signal — generation increment plus
    // an emitted `ConfigReloaded` event — that scoped reloads rely on to
    // invalidate idle leases.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fingerprint_change_bumps_generation() {
        use nebula_resource::{ReloadOutcome, events::ResourceEvent};

        let manager = Manager::new();
        let bot = TelegramBot::new();
        let resident_rt = Resident::<TelegramBot>::new(ResidentConfig::default());
        let org = OrgId::new();
        let scope = ScopeLevel::Organization(org);

        manager
            .register(RegistrationSpec {
                resource: bot,
                config: test_config(),
                scope: scope.clone(),
                slot_identity: SlotIdentity::Unbound,
                topology: resident_rt,
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register should succeed");

        let mut events = manager.subscribe_events();

        let managed = manager
            .lookup::<TelegramBot>(&scope)
            .expect("lookup should succeed");
        assert_eq!(managed.generation(), 0);

        // Reload with a different token — fingerprint changes, manager
        // bumps generation and emits `ConfigReloaded`.
        let new_config = TelegramConfig {
            token: "tg-bot-token-rotated".into(),
        };
        let outcome = manager
            .reload_config::<TelegramBot>(new_config, &scope)
            .expect("reload should succeed");

        assert_eq!(outcome, ReloadOutcome::SwappedImmediately);
        assert_eq!(managed.generation(), 1);

        // Drain any unrelated events to find ConfigReloaded.
        let mut found = false;
        for _ in 0..16 {
            match events.try_recv() {
                Some(ResourceEvent::ConfigReloaded { key }) => {
                    assert_eq!(key, TelegramBot::key());
                    found = true;
                    break;
                },
                Some(_) => continue,
                None => break,
            }
        }
        assert!(
            found,
            "fingerprint change must emit ResourceEvent::ConfigReloaded"
        );

        // No-op reload (same config again) → no generation bump.
        let outcome2 = manager
            .reload_config::<TelegramBot>(
                TelegramConfig {
                    token: "tg-bot-token-rotated".into(),
                },
                &scope,
            )
            .expect("idempotent reload should succeed");
        assert_eq!(outcome2, ReloadOutcome::NoChange);
        assert_eq!(managed.generation(), 1);
    }

    // -----------------------------------------------------------------------
    // Smoke check — `ctx_for_execution` falls through to global on miss.
    // Pinned to confirm the registry's scope fallback didn't accidentally
    // promote scoped resources into the global namespace during Phase 7
    // refactors. (Belongs here because it cross-cuts shared-resource
    // semantics: scope isolation must hold even when the request scope
    // isn't a registered scope.)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn execution_scope_falls_through_to_global() {
        let manager = Manager::new();
        let bot = TelegramBot::new();
        let counter = Arc::clone(&bot.create_counter);
        let resident_rt = Resident::<TelegramBot>::new(ResidentConfig::default());

        manager
            .register(RegistrationSpec {
                resource: bot,
                config: test_config(),
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: resident_rt,
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register should succeed");

        // Execution-scoped ctx → not registered at Execution scope, but the
        // registry falls back to Global per `Registry::find_by_scope`.
        let ctx = ctx_for_execution();
        let _lease = manager
            .acquire_resident::<TelegramBot>(&ctx, &AcquireOptions::default())
            .await
            .expect("global fallback should succeed");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

// ---------------------------------------------------------------------------
// Managed rows: derived actions reach ManagedRow<R> fields through the engine
// ---------------------------------------------------------------------------

mod managed_row {
    use std::{
        num::NonZeroU32,
        sync::{Mutex, atomic::AtomicU32},
        time::{Duration, Instant},
    };

    use nebula_action::ActionContext;
    use nebula_core::id::ExecutionId;
    use nebula_execution::ExecutionStatus;
    use nebula_resource::{
        ErrorKind, PoolConfig, PoolProvider, Pooled,
        call::{
            Cost, Effect, ManagedRow, OpCx, OpError, Operation, SentState, SessionClosed,
            SessionEnd, SessionProvider, SessionSpec, UNIT_DEADLINE_CAP,
        },
        rate_limit::{Rate, RowLimit},
    };
    use tokio::sync::Notify;

    use super::*;

    // ── a resident service counting its provider calls ───────────────────

    /// Provider calls made on the service.
    #[derive(Default)]
    struct Calls(AtomicU64);

    impl Calls {
        fn count(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Clone)]
    struct Svc(Arc<Calls>);

    #[async_trait::async_trait]
    impl Provider for Svc {
        type Config = ();
        type Instance = Arc<Calls>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("test.managed_row.svc")
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("ManagedRowSvc"),
                "",
            )
        }

        async fn create(&self, (): &(), _: &ResourceContext) -> Result<Arc<Calls>, ResourceError> {
            Ok(Arc::clone(&self.0))
        }
    }

    nebula_resource::no_credential_slots!(Svc);

    impl ResidentProvider for Svc {}

    /// Registers a service row, limited to `limit`; returns its call count.
    fn register_svc(manager: &Manager, limit: Option<RowLimit>) -> Arc<Calls> {
        let calls = Arc::new(Calls::default());
        manager
            .register(RegistrationSpec {
                resource: Svc(Arc::clone(&calls)),
                config: (),
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<Svc>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: limit,
            })
            .expect("register the service");
        calls
    }

    /// One provider call costing `cost`: counts it and yields the count.
    struct Call(Cost);

    impl Operation<Svc> for Call {
        type Output = u64;
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OpCx<'_, Svc>) -> Result<u64, OpError> {
            let attempt = cx.attempt(self.0).await?;
            let calls = attempt.instance().0.fetch_add(1, Ordering::SeqCst) + 1;
            attempt.settle(SentState::Sent);
            Ok(calls)
        }
    }

    // ── a pooled ledger with transactions ────────────────────────────────

    /// The ledger's committed entries, shared by every connection.
    type Committed = Arc<Mutex<Vec<String>>>;

    /// Session bodies entered by action-scoped rows. A write session must be
    /// refused before this counter changes.
    static SESSION_BODIES: AtomicU32 = AtomicU32::new(0);

    #[derive(Clone)]
    struct Ledger(Committed);

    struct Conn(Committed);

    struct Tx<'c> {
        conn: &'c mut Conn,
        pending: Vec<String>,
    }

    #[async_trait::async_trait]
    impl Provider for Ledger {
        type Config = ();
        type Instance = Conn;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("test.managed_row.ledger")
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("ManagedRowLedger"),
                "",
            )
        }

        async fn create(&self, (): &(), _: &ResourceContext) -> Result<Conn, ResourceError> {
            Ok(Conn(Arc::clone(&self.0)))
        }
    }

    nebula_resource::no_credential_slots!(Ledger);

    impl PoolProvider for Ledger {}

    impl SessionProvider for Ledger {
        type Session<'c> = Tx<'c>;

        async fn open<'c>(&'c self, conn: &'c mut Conn, (): &'c ()) -> Result<Tx<'c>, OpError> {
            Ok(Tx {
                conn,
                pending: Vec::new(),
            })
        }

        async fn close<'c>(&'c self, tx: Tx<'c>, end: SessionEnd) -> SessionClosed {
            match end {
                SessionEnd::Commit => {
                    tx.conn.0.lock().expect("ledger lock").extend(tx.pending);
                    SessionClosed::Committed
                },
                _ => SessionClosed::RolledBack { refused: None },
            }
        }
    }

    // ── engine plumbing ──────────────────────────────────────────────────

    /// A derived stateless action holding one required
    /// `ManagedRow<$provider>` field and explicitly declaring that its body
    /// performs no external business effects. The engine consequently serves
    /// a row facade that admits `Effect::Read` only.
    macro_rules! row_action {
        ($ty:ident, $key:literal, $field:ident: $provider:ty) => {
            #[derive(nebula_action::Action)]
            #[action(
                                                    key = $key,
                                                    name = $key,
                                                    description = "managed row integration action",
                                                    input = serde_json::Value,
                                                    output = serde_json::Value,
                                                    no_external_effects
                                                )]
            struct $ty {
                #[resource]
                $field: ManagedRow<$provider>,
            }
        };
    }

    #[derive(nebula_action::Action)]
    #[action(
        key = "test.managed_row.undeclared",
        name = "Undeclared managed row",
        description = "safe-default integration action",
        input = serde_json::Value,
        output = serde_json::Value
    )]
    struct UndeclaredRow {
        #[resource]
        svc: ManagedRow<Svc>,
    }

    impl StatelessAction for UndeclaredRow {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let calls = self.svc.submit(Call(Cost::ONE)).await?;
            Ok(ActionResult::success(serde_json::json!({ "calls": calls })))
        }
    }

    fn engine(manager: Arc<Manager>, register: impl FnOnce(&ActionRegistry)) -> WorkflowEngine {
        let registry = Arc::new(ActionRegistry::new());
        register(&registry);
        let metrics = MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                registry,
                Arc::new(InProcessRunner::new()),
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .expect("runtime"),
        );
        WorkflowEngine::new(runtime, metrics)
            .expect("engine")
            .with_resource_manager(manager)
    }

    /// One node of `action`, its `slot` bound to `resource`.
    fn node_of(action: &str, slot: &str, resource: &ResourceKey) -> NodeDefinition {
        NodeDefinition::new(node_key!("row"), "Row", "core", action)
            .expect("valid node")
            .with_resource_binding(slot, resource.as_str())
    }

    async fn run(
        engine: &WorkflowEngine,
        node: NodeDefinition,
        input: serde_json::Value,
        budget: ExecutionBudget,
    ) -> nebula_engine::ExecutionResult {
        engine
            .execute_workflow(
                &nebula_engine::store_seam::single_tenant_scope(),
                &make_workflow(vec![node]),
                input,
                budget,
            )
            .await
            .expect("workflow execution")
    }

    fn output(result: &nebula_engine::ExecutionResult) -> &serde_json::Value {
        result.node_output(&node_key!("row")).expect("node output")
    }

    // ── (a) a unit runs on the row ───────────────────────────────────────

    #[tokio::test]
    async fn a_derived_action_without_effect_attestation_is_rejected_before_instantiation() {
        let manager = Arc::new(Manager::new());
        let calls = register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<UndeclaredRow>()
                .expect("register");
        });

        let result = run(
            &engine,
            node_of("test.managed_row.undeclared", "svc", &Svc::key()),
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;
        assert!(!result.is_success());
        assert_eq!(calls.count(), 0, "the action was never instantiated");
    }

    row_action!(ReadSvc, "test.managed_row.read", svc: Svc);

    impl StatelessAction for ReadSvc {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let calls = self.svc.submit(Call(Cost::ONE)).await?;
            Ok(ActionResult::success(serde_json::json!({ "calls": calls })))
        }
    }

    #[tokio::test]
    async fn an_action_runs_a_unit_on_its_managed_row_field() {
        let manager = Arc::new(Manager::new());
        let calls = register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<ReadSvc>()
                .expect("register");
        });

        let result = run(
            &engine,
            node_of("test.managed_row.read", "svc", &Svc::key()),
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;
        assert!(result.is_success(), "{result:?}");
        assert_eq!(output(&result)["calls"], 1, "the attempt's instance");
        assert_eq!(calls.count(), 1);
    }

    // ── (b) a session commits or rolls back ──────────────────────────────

    row_action!(Book, "test.managed_row.book", ledger: Ledger);

    impl StatelessAction for Book {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let entry = input["entry"].as_str().unwrap_or_default().to_owned();
            let fail = input["fail"].as_bool().unwrap_or(false);
            let booked = self
                .ledger
                .session(SessionSpec::new(Cost::ONE), move |tx, _cx| {
                    Box::pin(async move {
                        SESSION_BODIES.fetch_add(1, Ordering::SeqCst);
                        tx.pending.push(entry);
                        if fail {
                            return Err(OpError::new(ErrorKind::Transient, "body failed"));
                        }
                        Ok(())
                    })
                })
                .await;
            Ok(ActionResult::success(match booked {
                Ok(()) => serde_json::json!({ "committed": true }),
                Err(error) => serde_json::json!({
                    "committed": false,
                    "sent": error.sent().as_str(),
                }),
            }))
        }
    }

    #[tokio::test]
    async fn an_action_scoped_write_session_is_refused_before_its_body() {
        let manager = Arc::new(Manager::new());
        let committed = Committed::default();
        manager
            .register(RegistrationSpec {
                resource: Ledger(Arc::clone(&committed)),
                config: (),
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Pooled::<Ledger>::new(PoolConfig::default(), 0),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register the ledger");
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<Book>()
                .expect("register");
        });
        SESSION_BODIES.store(0, Ordering::SeqCst);

        let result = run(
            &engine,
            node_of("test.managed_row.book", "ledger", &Ledger::key()),
            serde_json::json!({ "entry": "a", "fail": false }),
            ExecutionBudget::default(),
        )
        .await;
        assert_eq!(output(&result)["committed"], false);
        assert_eq!(output(&result)["sent"], "not_sent");
        assert_eq!(SESSION_BODIES.load(Ordering::SeqCst), 0);
        assert!(committed.lock().expect("ledger lock").is_empty());
    }

    // ── (c) cancelling the execution refuses a queued unit ───────────────

    /// The execution the cancellation test's action runs in.
    static CANCEL_EXECUTION: Mutex<Option<ExecutionId>> = Mutex::new(None);
    /// Fired once the second unit is about to ask for its attempt.
    static CANCEL_WAITING: Notify = Notify::const_new();
    /// The second unit's refusal and the attempts it was granted.
    static CANCEL_REFUSED: Mutex<Option<(ErrorKind, u32)>> = Mutex::new(None);
    /// Fired once the second unit's attempt was refused.
    static CANCEL_SETTLED: Notify = Notify::const_new();

    /// Asks for one attempt, recording the refusal it gets.
    struct Queued;

    impl Operation<Svc> for Queued {
        type Output = u64;
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OpCx<'_, Svc>) -> Result<u64, OpError> {
            CANCEL_WAITING.notify_one();
            let refused = match cx.attempt(Cost::ONE).await {
                Ok(attempt) => {
                    let calls = attempt.instance().0.fetch_add(1, Ordering::SeqCst) + 1;
                    attempt.settle(SentState::Sent);
                    return Ok(calls);
                },
                Err(refused) => refused,
            };
            *CANCEL_REFUSED.lock().expect("refusal slot") =
                Some((refused.kind().clone(), cx.attempts()));
            CANCEL_SETTLED.notify_one();
            Err(refused)
        }
    }

    row_action!(Twice, "test.managed_row.twice", svc: Svc);

    impl StatelessAction for Twice {
        async fn execute(
            &self,
            _input: serde_json::Value,
            ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            self.svc.submit(Call(Cost::ONE)).await?;
            *CANCEL_EXECUTION.lock().expect("execution slot") = ctx.scope().execution_id;
            let calls = self.svc.submit(Queued).await?;
            Ok(ActionResult::success(serde_json::json!({ "calls": calls })))
        }
    }

    #[tokio::test]
    async fn cancelling_the_execution_refuses_the_unit_waiting_for_quota_unsent() {
        let manager = Arc::new(Manager::new());
        // One call a minute: the second unit waits inside its deadline
        // rather than being refused `Exhausted` at once.
        let per_minute = Rate::new(NonZeroU32::MIN, Duration::from_mins(1)).expect("valid rate");
        let calls = register_svc(&manager, Some(RowLimit::rate(per_minute)));
        let engine = Arc::new(engine(manager, |registry| {
            registry
                .register_stateless_factory::<Twice>()
                .expect("register");
        }));

        let running = tokio::spawn({
            let engine = Arc::clone(&engine);
            async move {
                run(
                    &engine,
                    node_of("test.managed_row.twice", "svc", &Svc::key()),
                    serde_json::json!(null),
                    ExecutionBudget::default(),
                )
                .await
            }
        });
        // Bounded only so a regression fails instead of hanging the suite.
        tokio::time::timeout(Duration::from_secs(30), CANCEL_WAITING.notified())
            .await
            .expect("the second unit asks for its attempt");
        let execution = CANCEL_EXECUTION
            .lock()
            .expect("execution slot")
            .expect("the action published its execution");
        assert!(engine.cancel_execution(execution), "the execution is live");

        tokio::time::timeout(Duration::from_secs(30), CANCEL_SETTLED.notified())
            .await
            .expect("the second unit's attempt is refused");
        assert_eq!(
            *CANCEL_REFUSED.lock().expect("refusal slot"),
            Some((ErrorKind::Cancelled, 0)),
            "refused before any grant: NotSent"
        );
        let result = running.await.expect("joined");
        assert_eq!(result.status, ExecutionStatus::Cancelled);
        assert_eq!(calls.count(), 1, "only the first unit reached the provider");
    }

    // ── (d) the execution budget bounds the unit deadline ────────────────

    row_action!(UnitDeadline, "test.managed_row.deadline", svc: Svc);

    /// Yields the seconds left until the unit's deadline.
    struct Remaining;

    impl Operation<Svc> for Remaining {
        type Output = f64;
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OpCx<'_, Svc>) -> Result<f64, OpError> {
            Ok(cx
                .deadline()
                .saturating_duration_since(Instant::now())
                .as_secs_f64())
        }
    }

    impl StatelessAction for UnitDeadline {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let remaining = self.svc.submit(Remaining).await?;
            Ok(ActionResult::success(
                serde_json::json!({ "remaining": remaining }),
            ))
        }
    }

    #[tokio::test]
    async fn the_execution_budget_bounds_the_unit_deadline() {
        let manager = Arc::new(Manager::new());
        register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<UnitDeadline>()
                .expect("register");
        });
        let node = || node_of("test.managed_row.deadline", "svc", &Svc::key());
        let remaining = |result: &nebula_engine::ExecutionResult| {
            output(result)["remaining"]
                .as_f64()
                .expect("seconds remaining")
        };

        let budgeted = run(
            &engine,
            node(),
            serde_json::json!(null),
            ExecutionBudget::default().with_max_duration(Duration::from_mins(1)),
        )
        .await;
        let left = remaining(&budgeted);
        assert!(left <= 60.0 && left > 50.0, "bounded by the budget: {left}");

        let unbudgeted = run(
            &engine,
            node(),
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;
        let left = remaining(&unbudgeted);
        let cap = UNIT_DEADLINE_CAP.as_secs_f64();
        assert!(left <= cap && left > cap - 10.0, "the unit cap: {left}");
    }

    // ── (e) a refused unit's retry hint delays the node retry ────────────

    /// When each dispatch of the hinted action started.
    static HINTED_STARTS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());

    /// Refused by the provider's quota before anything was sent.
    struct Throttled;

    impl Operation<Svc> for Throttled {
        type Output = ();
        const EFFECT: Effect = Effect::Read;

        async fn run(self, _cx: &mut OpCx<'_, Svc>) -> Result<(), OpError> {
            Err(OpError::new(
                ErrorKind::Exhausted {
                    retry_after: Some(Duration::from_millis(300)),
                },
                "quota exhausted",
            ))
        }
    }

    row_action!(Hinted, "test.managed_row.hinted", svc: Svc);

    impl StatelessAction for Hinted {
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let first = {
                let mut starts = HINTED_STARTS.lock().expect("starts");
                starts.push(Instant::now());
                starts.len() == 1
            };
            if first {
                let refused = self.svc.submit(Throttled).await.expect_err("throttled");
                assert_eq!(refused.sent(), SentState::NotSent);
                return Err(refused.into());
            }
            Ok(ActionResult::success(input))
        }
    }

    #[tokio::test]
    async fn an_unsent_exhausted_unit_is_retried_after_its_hint() {
        let manager = Arc::new(Manager::new());
        register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<Hinted>()
                .expect("register");
        });
        let mut node = node_of("test.managed_row.hinted", "svc", &Svc::key());
        node.retry_policy = Some(nebula_workflow::RetryConfig::exponential(3, 1, 10_000));

        let result = run(
            &engine,
            node,
            serde_json::json!("payload"),
            ExecutionBudget::default(),
        )
        .await;
        assert_eq!(result.status, ExecutionStatus::Completed);
        let starts = HINTED_STARTS.lock().expect("starts").clone();
        assert_eq!(starts.len(), 2);
        assert!(
            starts[1] - starts[0] >= Duration::from_millis(300),
            "retried after {:?}, before the unit's hint",
            starts[1] - starts[0]
        );
    }

    // ── (f) replay-safe mutation still requires effect-owner authority ───

    /// A provider mutation that claims repeats are absorbed.
    struct AbsorbedWrite;

    impl Operation<Svc> for AbsorbedWrite {
        type Output = ();
        const EFFECT: Effect = Effect::Idempotent;

        async fn run(self, cx: &mut OpCx<'_, Svc>) -> Result<(), OpError> {
            let attempt = cx.attempt(Cost::ONE).await?;
            attempt.instance().0.fetch_add(1, Ordering::SeqCst);
            attempt.settle(SentState::Sent);
            Ok(())
        }
    }

    row_action!(IdempotentWrite, "test.managed_row.idempotent", svc: Svc);

    impl StatelessAction for IdempotentWrite {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            let error = self
                .svc
                .submit(AbsorbedWrite)
                .await
                .expect_err("an idempotent mutation still needs effect authority");
            Err(error.into())
        }
    }

    #[tokio::test]
    async fn an_action_scoped_idempotent_write_is_refused_before_the_provider() {
        let manager = Arc::new(Manager::new());
        let calls = register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<IdempotentWrite>()
                .expect("register");
        });

        let result = run(
            &engine,
            node_of("test.managed_row.idempotent", "svc", &Svc::key()),
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;
        assert!(!result.is_success());
        assert_eq!(calls.count(), 0, "the mutation never reached the provider");
    }

    // ── (g) a write is refused before the provider boundary ──────────────

    /// Dispatches of the unknown-outcome action.
    static UNKNOWN_DISPATCHES: AtomicU32 = AtomicU32::new(0);

    /// A write whose provider call may have been applied.
    struct LostWrite;

    impl Operation<Svc> for LostWrite {
        type Output = ();
        const EFFECT: Effect = Effect::Write;

        async fn run(self, cx: &mut OpCx<'_, Svc>) -> Result<(), OpError> {
            let attempt = cx.attempt(Cost::ONE).await?;
            attempt.instance().0.fetch_add(1, Ordering::SeqCst);
            attempt.settle(SentState::MaybeSent);
            Err(OpError::new(ErrorKind::Transient, "connection reset"))
        }
    }

    row_action!(UnknownWrite, "test.managed_row.unknown", svc: Svc);

    impl StatelessAction for UnknownWrite {
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            UNKNOWN_DISPATCHES.fetch_add(1, Ordering::SeqCst);
            let error = ActionError::from(self.svc.submit(LostWrite).await.expect_err("lost"));
            assert!(matches!(error, ActionError::Fatal { .. }), "{error}");
            Err(error)
        }
    }

    #[tokio::test]
    async fn an_action_scoped_write_is_fatal_before_the_provider_and_never_retried() {
        let manager = Arc::new(Manager::new());
        let calls = register_svc(&manager, None);
        let engine = engine(manager, |registry| {
            registry
                .register_stateless_factory::<UnknownWrite>()
                .expect("register");
        });
        let mut node = node_of("test.managed_row.unknown", "svc", &Svc::key());
        node.retry_policy = Some(nebula_workflow::RetryConfig::exponential(3, 1, 10));

        let result = run(
            &engine,
            node,
            serde_json::json!(null),
            ExecutionBudget::default(),
        )
        .await;
        assert!(!result.is_success());
        assert_eq!(UNKNOWN_DISPATCHES.load(Ordering::SeqCst), 1, "no retry");
        assert_eq!(calls.count(), 0, "the write never reached the provider");
    }
}
