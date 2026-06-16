//! Engine holds the `kind → typed factory` allowlist in its state and
//! exposes it through an accessor, populated from the typed factories a
//! composition root builds for the resources an `impl Plugin` declares.
//!
//! ## What this pins
//!
//! `impl Plugin` is the runtime source of truth for *what* registers
//! (`actions()` / `resources()` / `credentials()` — INTEGRATION_MODEL,
//! "Plugin packaging" §). `Plugin::resources()` yields
//! `Vec<Arc<dyn nebula_resource::ResourceFactory>>` — the B+ merged
//! contribution contract (ADR-0095 D2). `ResourceFactory` carries both
//! the introspection arm (`key`, `metadata`, `validate`) and the
//! construction arm (`register`).
//!
//! The `kind → typed factory` allowlist is populated by reflecting over
//! `Plugin::resources()` and inserting each factory by its `key()`. The
//! composition root may also build factories manually via
//! [`nebula_engine::KindActivator`] and insert them directly into a
//! [`nebula_engine::ResourceActivatorRegistry`].
//!
//! This test pins the honest, wired behavior: a plugin declares a resource
//! of kind `demo.widget` via `ResourceFactory`; the composition root builds
//! the typed registry and hands it to the engine; the engine holds it and
//! the accessor resolves the wired kind (and rejects an undeclared one).

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, ResourceKey, action_key, node_key, resource_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
    KindActivator, Plugin, PluginManifest, PluginRegistry, ResolvedPlugin,
    ResourceActivatorRegistry, WorkflowEngine,
};
use nebula_metrics::MetricsRegistry;
use nebula_resource::Resident;
use nebula_resource::{
    ScopeLevel,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    topology::resident,
};
use nebula_schema::HasSchema;

// ── A resource the plugin declares (kind = "demo.widget") ───────────────────

#[derive(Debug, Clone)]
struct DemoError(String);

impl std::fmt::Display for DemoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DemoError {}

impl From<DemoError> for ResourceError {
    fn from(e: DemoError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
struct DemoConfig {
    #[serde(default)]
    label: String,
}

nebula_schema::impl_empty_has_schema!(DemoConfig);

impl ResourceConfig for DemoConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.label.is_empty() {
            return Err(ResourceError::permanent("label must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone)]
struct DemoResource {
    create_counter: Arc<AtomicU64>,
}

impl DemoResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for DemoResource {
    type Config = DemoConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("demo.widget")
    }

    async fn create(
        &self,
        _config: &DemoConfig,
        _ctx: &nebula_resource::ResourceContext,
    ) -> Result<Arc<AtomicU64>, nebula_resource::Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(
            <Self as Provider>::key(),
            "demo.widget".to_owned(),
            String::new(),
            <DemoConfig as HasSchema>::schema(),
        )
    }
}

impl nebula_core::DeclaresDependencies for DemoResource {}

impl nebula_resource::HasCredentialSlots for DemoResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl resident::ResidentProvider for DemoResource {
    fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
        runtime.load(Ordering::Relaxed) < u64::MAX
    }
}

// ── A plugin that declares the resource (canon: impl Plugin is the
//    runtime source of truth for what registers) ─────────────────────────────

#[derive(Debug)]
struct DemoPlugin(PluginManifest);

impl Plugin for DemoPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.0
    }

    /// Returns the B+ merged `ResourceFactory` for `demo.widget` (ADR-0095 D2).
    ///
    /// The factory carries both the introspection arm (`key`, `metadata`,
    /// `validate`) and the construction arm (`register`), supplied here via
    /// a `KindActivator` with a resident topology.
    fn resources(&self) -> Vec<Arc<dyn nebula_resource::ResourceFactory>> {
        vec![Arc::new(KindActivator::<DemoResource, _, _>::new(
            DemoResource::new,
            || Resident::<DemoResource>::new(resident::config::Config::default()),
        ))]
    }
}

fn demo_plugin_registry() -> PluginRegistry {
    // `demo` is the plugin key; the declared resource key is namespaced
    // `demo.widget` (ResolvedPlugin enforces the `{plugin}.` prefix).
    let manifest = PluginManifest::builder("demo", "Demo")
        .build()
        .expect("manifest builds");
    let resolved = ResolvedPlugin::from(DemoPlugin(manifest)).expect("resolves");
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(resolved))
        .expect("plugin registers");
    registry
}

/// Build the typed registrar the composition root would assemble for the
/// `demo.widget` resource the plugin declares. The `kind` string is taken
/// from the resource's own catalog key (`AnyResource::key()`) — never
/// guessed — and the topology is the one a `demo.widget` deployment uses
/// (resident); both are supplied by the typed constructors the plugin
/// author holds, NOT synthesized from the erased `AnyResource`.
fn demo_registrars(plugins: &PluginRegistry) -> ResourceActivatorRegistry {
    let mut registrars = ResourceActivatorRegistry::new();

    // The kind comes from the plugin-declared resource's catalog key.
    let kind = plugins
        .all_resources()
        .map(|(_pk, r)| r.key())
        .find(|k| k.as_str() == "demo.widget")
        .expect("plugin declares the demo.widget resource");

    registrars.insert(
        kind.as_str().to_owned(),
        Arc::new(KindActivator::<DemoResource, _, _>::new(
            DemoResource::new,
            || Resident::<DemoResource>::new(resident::config::Config::default()),
        )),
    );
    registrars
}

// ── Minimal engine harness (mirrors resource_integration.rs) ────────────────

struct NoopHandler;

impl Action for NoopHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(action_key!("test.noop.static"), "Noop", "static")
    }
    fn dependencies() -> &'static Dependencies {
        use std::sync::OnceLock;
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for NoopHandler {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

fn build_engine(registrars: ResourceActivatorRegistry) -> WorkflowEngine {
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        ActionMetadata::new(action_key!("test.noop"), "Noop", "noop"),
        NoopHandler,
    );
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
        .expect("runtime builds"),
    );
    WorkflowEngine::new(runtime, metrics)
        .expect("engine builds")
        .with_resource_registrars(registrars)
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Headline: a plugin declares a resource of kind `demo.widget`; the
/// composition root builds the typed registrar for it and threads the
/// registry into the engine. The engine holds the allowlist in its state
/// and the accessor resolves the wired kind.
#[tokio::test]
async fn engine_holds_registrars_built_for_plugin_declared_resource() {
    let plugins = demo_plugin_registry();
    let registrars = demo_registrars(&plugins);
    let engine = build_engine(registrars);

    // The engine exposes the allowlist and the plugin-declared kind is
    // present (wired through the explicit typed-registration path).
    assert!(
        engine.resource_registrars().contains("demo.widget"),
        "the kind the plugin declared must be registrable through the \
         engine's allowlist"
    );

    // A kind no plugin declared and no registrar was wired for is absent —
    // fail-closed, never a silent match.
    assert!(
        !engine.resource_registrars().contains("not-declared"),
        "an undeclared kind must not be in the allowlist"
    );

    assert_eq!(engine.resource_registrars().len(), 1);
}

/// The wired registrar actually performs a typed registration end to end
/// (it is a *live* registrar, not a placeholder): driving it through the
/// allowlist registers the resource against a `Manager` under its key.
#[tokio::test]
async fn wired_registrar_performs_typed_registration() {
    use nebula_engine::RegisterRequest;
    use nebula_expression::ExpressionEngine;
    use nebula_resource::Manager;

    let plugins = demo_plugin_registry();
    let registrars = demo_registrars(&plugins);
    let engine = build_engine(registrars);

    let manager = Manager::new();
    let expr = ExpressionEngine::with_cache_size(16);

    engine
        .register_resource(
            "demo.widget",
            &manager,
            RegisterRequest {
                config_json: serde_json::json!({ "label": "wired" }),
                expr_engine: &expr,
                slot_bindings: HashMap::new(),
                credential_ids: HashMap::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
        )
        .await
        .expect("the plugin-declared kind registers via the typed manager call");

    assert!(
        manager
            .get_any(&<DemoResource as Provider>::key(), &ScopeLevel::Global)
            .is_some(),
        "the plugin-declared resource must be resolvable in the manager \
         after registration"
    );
}

/// Default engine (no registrars threaded) holds an empty, fail-closed
/// allowlist — every kind is rejected. Pins that the registry is always
/// present in engine state (not an `Option`) and defaults closed.
#[tokio::test]
async fn default_engine_has_empty_failclosed_registry() {
    let engine = build_engine(ResourceActivatorRegistry::new());
    assert!(engine.resource_registrars().is_empty());
    assert!(!engine.resource_registrars().contains("demo.widget"));
}

/// Smoke: wiring registrars does not regress ordinary dispatch — a trivial
/// single-node workflow still runs on an engine that carries a populated
/// allowlist.
#[tokio::test]
async fn registrars_do_not_regress_dispatch() {
    use nebula_core::id::WorkflowId;
    use nebula_execution::context::ExecutionBudget;
    use nebula_workflow::{
        CURRENT_SCHEMA_VERSION, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };

    let plugins = demo_plugin_registry();
    let engine = build_engine(demo_registrars(&plugins));

    let now = chrono::Utc::now();
    let node = node_key!("n");
    let wf = WorkflowDefinition {
        id: WorkflowId::new(),
        name: "registrar-smoke".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![NodeDefinition::new(node.clone(), "N", "core", "test.noop").expect("node")],
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

    let result = engine
        .execute_workflow(
            &wf,
            serde_json::json!({ "ok": true }),
            ExecutionBudget::default(),
        )
        .await
        .expect("workflow executes");
    assert!(result.is_success(), "dispatch must not regress");
}
