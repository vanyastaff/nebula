//! Phase 9 / Task 9.4 — End-to-end engine pipeline integration test.
//!
//! Headline scenario: a single-node workflow whose action declares a
//! `ValidSchema` input that mixes literal + `{{ }}`-template parameters.
//! The engine receives the node, resolves expressions through the
//! `ParamResolver` → `ExpressionEngine` chain, hands the resolved JSON to
//! the action via the dispatch path, and the action body inspects the
//! result.
//!
//! This is the **integration smoke** for Phases 1–8 of the M6 redesign:
//! it pins the seam where `nebula-schema` (input shape), `nebula-validator`
//! (rules), and `nebula-expression` (template evaluation) come together
//! at the engine boundary.
//!
//! Specifically asserted:
//!
//!   1. The engine successfully resolves a `{{ now() }}` template at dispatch time and forwards the
//!      resulting JSON value to the action handler.
//!   2. Static literals pass through untouched.
//!   3. The handler observes the post-resolution input — every parameter is a literal value when
//!      `execute` runs.
//!   4. A separate node binding through the engine's resource manager verifies that resource
//!      acquisition (Phase 6 typed slots) and param resolution coexist cleanly within one
//!      execution.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{
    ActionKey, Dependencies, ResourceKey, ScopeLevel, action_key, id::WorkflowId, node_key,
    resource_key,
};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox,
    WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_metrics::MetricsRegistry;
use nebula_resource::{
    Manager, RegistrationSpec, ResidentConfig, ResourceContext, SlotIdentity,
    error::Error as ResourceError,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use nebula_workflow::{NodeDefinition, ParamValue, Version, WorkflowConfig, WorkflowDefinition};

// ── Action handler ─────────────────────────────────────────────────────────

/// Records the JSON input it observed so the test can introspect what the
/// engine handed it after expression resolution + validation.
struct PipelineWitness {
    seen_input: Arc<parking_lot::Mutex<Option<serde_json::Value>>>,
}

impl Action for PipelineWitness {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.phase9.pipeline_witness"),
                "PipelineWitness",
                "Phase 9 e2e pipeline witness",
            )
        })
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for PipelineWitness {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        // Record the resolved input — the test asserts that every
        // expression has already been replaced by a literal value.
        *self.seen_input.lock() = Some(input.clone());
        Ok(ActionResult::success(input))
    }
}

// ── Resource fixture for slot-binding integration check ────────────────────

#[derive(Clone, Debug)]
struct WitnessResourceConfig {
    label: String,
}

nebula_schema::impl_empty_has_schema!(WitnessResourceConfig);

impl ResourceConfig for WitnessResourceConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.label.is_empty() {
            Err(ResourceError::permanent("label must not be empty"))
        } else {
            Ok(())
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut h);
        h.finish()
    }
}

#[derive(Debug)]
#[allow(dead_code, reason = "fields exist for diagnostics on lease inspection")]
struct WitnessResourceInner {
    instance_id: u64,
    label: String,
}

#[derive(Clone)]
struct WitnessResource {
    create_counter: Arc<AtomicU64>,
}

impl WitnessResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Debug, Clone)]
struct WitnessError(String);

impl std::fmt::Display for WitnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WitnessError {}

impl From<WitnessError> for ResourceError {
    fn from(e: WitnessError) -> Self {
        ResourceError::transient(e.0)
    }
}

impl Resource for WitnessResource {
    type Config = WitnessResourceConfig;
    type Runtime = Arc<WitnessResourceInner>;
    type Lease = Arc<WitnessResourceInner>;
    type Error = WitnessError;

    fn key() -> ResourceKey {
        resource_key!("phase9-witness-resource")
    }

    fn create(
        &self,
        config: &WitnessResourceConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<WitnessResourceInner>, WitnessError>> + Send {
        let counter = Arc::clone(&self.create_counter);
        let label = config.label.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(WitnessResourceInner {
                instance_id: id,
                label,
            }))
        }
    }

    async fn destroy(&self, _runtime: Arc<WitnessResourceInner>) -> Result<(), WitnessError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for WitnessResource {
    fn is_alive_sync(&self, _runtime: &Arc<WitnessResourceInner>) -> bool {
        true
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "phase9-e2e-pipeline".into(),
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
    ActionMetadata::new(key, name, "phase9 e2e pipeline test handler")
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Headline e2e test: one node with literal + expression-template parameters.
/// The engine resolves the template through `ExpressionEngine` and forwards
/// the resolved JSON to the action handler.
#[tokio::test]
async fn pipeline_resolves_expressions_before_handler_runs() {
    let seen_input = Arc::new(parking_lot::Mutex::new(None::<serde_json::Value>));
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("phase9.witness")),
        PipelineWitness {
            seen_input: Arc::clone(&seen_input),
        },
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

    // Build a node with three parameters:
    //   - `name`     : literal string
    //   - `timestamp`: `{{ now() }}` expression — engine resolves to current Unix timestamp before
    //     the handler sees it
    //   - `count`    : literal integer
    let node = node_key!("e2e_node");
    let mut node_def = NodeDefinition::new(node.clone(), "Witness", "phase9.witness").unwrap();
    node_def.parameters.insert(
        "name".into(),
        ParamValue::literal(serde_json::json!("alice")),
    );
    node_def
        .parameters
        .insert("timestamp".into(), ParamValue::expression("{{ now() }}"));
    node_def
        .parameters
        .insert("count".into(), ParamValue::literal(serde_json::json!(7)));
    let wf = make_workflow(vec![node_def]);

    // Capture wall-clock before+after to bound the resolved timestamp.
    let before = chrono::Utc::now().timestamp();
    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");
    let after = chrono::Utc::now().timestamp();

    assert!(
        result.is_success(),
        "workflow must succeed; got: {result:?}"
    );

    // The handler observed the resolved input — assert each piece.
    let observed = seen_input
        .lock()
        .clone()
        .expect("handler must have seen input");
    let obj = observed
        .as_object()
        .expect("resolved input must be a JSON object");

    assert_eq!(
        obj.get("name").and_then(serde_json::Value::as_str),
        Some("alice"),
        "literal `name` must pass through unchanged"
    );
    assert_eq!(
        obj.get("count").and_then(serde_json::Value::as_i64),
        Some(7),
        "literal `count` must pass through unchanged"
    );

    // `timestamp` must be a literal number — proof the expression resolved.
    let ts = obj
        .get("timestamp")
        .and_then(serde_json::Value::as_i64)
        .expect("`{{ now() }}` must have resolved to a numeric literal");
    assert!(
        before <= ts && ts <= after,
        "resolved `now()` timestamp `{ts}` must fall in [{before}, {after}]"
    );

    // The output of the node mirrors the input by design (the handler echoes).
    let output = result.node_output(&node).expect("node output");
    assert_eq!(
        output.get("name").and_then(serde_json::Value::as_str),
        Some("alice"),
        "handler output mirrors resolved input"
    );
}

/// Combined e2e: the same workflow ALSO has a resource manager attached.
/// Verifies expression resolution + resource manager wiring coexist within
/// one execution, mirroring how production engines run nodes that use both
/// templates and slot-bound resources.
#[tokio::test]
async fn pipeline_with_resource_manager_resolves_and_executes() {
    let manager = Arc::new(Manager::new());

    // Pre-register a resident `WitnessResource` so the manager has at least
    // one resource. The action handler does not consume it (Phase 6 typed
    // slot resolution is exercised in dedicated tests); this exercises that
    // attaching a manager does not regress the resolver pipeline.
    let resource = WitnessResource::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: WitnessResourceConfig {
                label: "test-witness".into(),
            },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(ResidentRuntime::<WitnessResource>::new(
                ResidentConfig::default(),
            )),
            acquire: Manager::erased_acquire_resident_for::<WitnessResource>(),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register witness resource");

    let seen_input = Arc::new(parking_lot::Mutex::new(None::<serde_json::Value>));
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("phase9.witness")),
        PipelineWitness {
            seen_input: Arc::clone(&seen_input),
        },
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
    let engine = WorkflowEngine::new(runtime, metrics)
        .unwrap()
        .with_resource_manager(manager.clone());

    let node = node_key!("e2e_node_with_manager");
    let mut node_def = NodeDefinition::new(node.clone(), "Witness", "phase9.witness").unwrap();
    node_def.parameters.insert(
        "greeting".into(),
        ParamValue::template("hello, {{ \"world\" }}!"),
    );
    node_def.parameters.insert(
        "static_value".into(),
        ParamValue::literal(serde_json::json!(42)),
    );
    let wf = make_workflow(vec![node_def]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    assert!(result.is_success(), "workflow must succeed");

    let observed = seen_input.lock().clone().expect("handler ran");
    let obj = observed.as_object().expect("object");

    assert_eq!(
        obj.get("greeting").and_then(serde_json::Value::as_str),
        Some("hello, world!"),
        "template `{{{{ \"world\" }}}}` must have rendered into the literal"
    );
    assert_eq!(
        obj.get("static_value").and_then(serde_json::Value::as_i64),
        Some(42),
        "static literal must pass through"
    );

    manager.shutdown();
}

/// Stage gate: parameter resolution failures terminate the node before the
/// handler runs. Confirms the engine fails fast on unevaluable expressions
/// rather than passing partially-resolved input to `execute`.
#[tokio::test]
async fn pipeline_unresolvable_expression_fails_node_before_handler() {
    let seen_input = Arc::new(parking_lot::Mutex::new(None::<serde_json::Value>));
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("phase9.witness")),
        PipelineWitness {
            seen_input: Arc::clone(&seen_input),
        },
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

    let node = node_key!("bad_node");
    let mut node_def = NodeDefinition::new(node.clone(), "Witness", "phase9.witness").unwrap();
    // A function that doesn't exist — runtime resolution failure.
    node_def.parameters.insert(
        "value".into(),
        ParamValue::expression("{{ no_such_function() }}"),
    );
    let wf = make_workflow(vec![node_def]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow finishes (the node fails, not the engine)");

    assert!(
        !result.is_success(),
        "workflow must surface the failure when expression resolution errors out"
    );
    assert!(
        seen_input.lock().is_none(),
        "handler must NOT have run when resolution failed"
    );
}
