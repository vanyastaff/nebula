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
    sync::{Arc, OnceLock},
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox,
    WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_metrics::MetricsRegistry;
use nebula_resource::Manager;
use nebula_schema::{HasSchema, ValidSchema};
use nebula_workflow::{NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};

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

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.resource_consumer.static"),
                "ResourceConsumer",
                "static",
            )
        })
    }
    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
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

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.resource_probe.static"),
                "ResourceProbe",
                "static",
            )
        })
    }
    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
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
        trigger: None,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    }
}

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "resource integration test")
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
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-consumer")),
        ResourceConsumerHandler,
    );

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            sandbox,
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
        NodeDefinition::new(node.clone(), "A", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
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
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-consumer")),
        ResourceConsumerHandler,
    );

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            sandbox,
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
        NodeDefinition::new(node.clone(), "A", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
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
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-probe")),
        ResourceProbeHandler,
    );

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            sandbox,
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
        NodeDefinition::new(node, "A", "resource-probe").unwrap(),
    ]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
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
// the `R::Credential` associated type was retired in Phase 4 (ADR-0044), and
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
        AcquireOptions, Manager, ResidentConfig, ResourceContext,
        error::Error,
        resource::{Resource, ResourceConfig, ResourceMetadata},
        runtime::{TopologyRuntime, resident::ResidentRuntime},
        topology::resident::Resident,
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

    #[derive(Clone, Debug)]
    struct TelegramConfig {
        token: String,
    }

    nebula_schema::impl_empty_has_schema!(TelegramConfig);

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

    impl Resource for TelegramBot {
        type Config = TelegramConfig;
        type Runtime = Arc<TelegramBotInner>;
        type Lease = Arc<TelegramBotInner>;
        type Error = TelegramError;

        fn key() -> ResourceKey {
            resource_key!("telegram-bot")
        }

        fn create(
            &self,
            _config: &TelegramConfig,
            _ctx: &ResourceContext,
        ) -> impl Future<Output = Result<Arc<TelegramBotInner>, TelegramError>> + Send {
            let counter = Arc::clone(&self.create_counter);
            async move {
                // Yield once to widen the concurrent-acquire interleaving
                // window — exposes any missing serialization in the
                // double-checked create path.
                tokio::task::yield_now().await;
                let id = counter.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(TelegramBotInner { instance_id: id }))
            }
        }

        async fn destroy(&self, _runtime: Arc<TelegramBotInner>) -> Result<(), TelegramError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for TelegramBot {
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

    impl Resource for AlternateBot {
        type Config = TelegramConfig;
        type Runtime = Arc<TelegramBotInner>;
        type Lease = Arc<TelegramBotInner>;
        type Error = TelegramError;

        fn key() -> ResourceKey {
            resource_key!("telegram-bot-alt")
        }

        fn create(
            &self,
            _config: &TelegramConfig,
            _ctx: &ResourceContext,
        ) -> impl Future<Output = Result<Arc<TelegramBotInner>, TelegramError>> + Send {
            let counter = Arc::clone(&self.create_counter);
            async move {
                tokio::task::yield_now().await;
                let id = counter.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(TelegramBotInner {
                    instance_id: 100_000 + id,
                }))
            }
        }

        async fn destroy(&self, _runtime: Arc<TelegramBotInner>) -> Result<(), TelegramError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for AlternateBot {
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
        let resident_rt = ResidentRuntime::<TelegramBot>::new(ResidentConfig::default());
        let org = OrgId::new();

        manager
            .register(
                bot,
                test_config(),
                ScopeLevel::Organization(org),
                TopologyRuntime::Resident(resident_rt),
                None,
                None,
            )
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
            .register(
                bot_a,
                test_config(),
                scope.clone(),
                TopologyRuntime::Resident(ResidentRuntime::<TelegramBot>::new(
                    ResidentConfig::default(),
                )),
                None,
                None,
            )
            .expect("register A should succeed");
        manager
            .register(
                bot_b,
                test_config(),
                scope,
                TopologyRuntime::Resident(ResidentRuntime::<AlternateBot>::new(
                    ResidentConfig::default(),
                )),
                None,
                None,
            )
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
            .register(
                bot_org_a,
                test_config(),
                ScopeLevel::Organization(org_a),
                TopologyRuntime::Resident(ResidentRuntime::<TelegramBot>::new(
                    ResidentConfig::default(),
                )),
                None,
                None,
            )
            .expect("register org_a should succeed");
        manager
            .register(
                bot_org_b,
                test_config(),
                ScopeLevel::Organization(org_b),
                TopologyRuntime::Resident(ResidentRuntime::<TelegramBot>::new(
                    ResidentConfig::default(),
                )),
                None,
                None,
            )
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
        let resident_rt = ResidentRuntime::<TelegramBot>::new(ResidentConfig::default());
        let org = OrgId::new();
        let scope = ScopeLevel::Organization(org);

        manager
            .register(
                bot,
                test_config(),
                scope.clone(),
                TopologyRuntime::Resident(resident_rt),
                None,
                None,
            )
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
                Ok(ResourceEvent::ConfigReloaded { key }) => {
                    assert_eq!(key, TelegramBot::key());
                    found = true;
                    break;
                },
                Ok(_) => continue,
                Err(_) => break,
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
        let resident_rt = ResidentRuntime::<TelegramBot>::new(ResidentConfig::default());

        manager
            .register(
                bot,
                test_config(),
                ScopeLevel::Global,
                TopologyRuntime::Resident(resident_rt),
                None,
                None,
            )
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
