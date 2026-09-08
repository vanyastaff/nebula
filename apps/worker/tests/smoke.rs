//! Smoke test for a manually assembled core-flavor runtime over in-memory
//! stores.
//!
//! Proves this explicit test assembly:
//!   `compose::build_core_flavor_runtime` (wires CorePlugin + in-memory stores)
//!   → owner materializes the execution and exact contract with a `Start` command
//!   → `WorkerRuntime::spawn` → control consumer → `WorkflowEngine::resume_execution`
//!   → execution reaches `Completed`.
//!
//! The test uses seeded in-memory adapters, so it does not boot the worker
//! binary or exercise its default SQLite composition.
//!
//! Red-ability: without `with_plugin` the engine cannot dispatch `core.set_fields`
//! and the execution never reaches `Completed`. The assertion on `completed` will
//! fire, producing a distinct failure message naming the missing plugin wire.

#[cfg(feature = "runtime-repair-red")]
use std::time::Instant;
use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::Utc;
#[cfg(feature = "runtime-repair-red")]
use nebula_core::accessor::Clock;
use nebula_core::{WorkflowId, id::ExecutionId, node_key};
use nebula_execution::ExecutionStatus;
use nebula_storage::{
    InMemoryControlQueue, InMemoryExecutionStore, InMemoryWorkflowStore,
    InMemoryWorkflowVersionStore,
};
use nebula_storage_port::{
    Scope,
    store::{ExecutionStore, NodeResultStore, StartAcceptanceStore, WorkflowStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, Version, WorkflowConfig,
    WorkflowDefinition,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use nebula_worker_bin::compose::build_core_flavor_runtime;
#[cfg(feature = "runtime-repair-red")]
use nebula_worker_bin::compose::build_core_flavor_runtime_for_runtime_repair_red;

#[cfg(feature = "runtime-repair-red")]
#[derive(Debug)]
struct FixedEvidenceClock {
    wall_time: chrono::DateTime<Utc>,
    monotonic: Instant,
}

#[cfg(feature = "runtime-repair-red")]
impl Clock for FixedEvidenceClock {
    fn now(&self) -> chrono::DateTime<Utc> {
        self.wall_time
    }

    fn monotonic(&self) -> Instant {
        self.monotonic
    }
}

// ── Scope used across these tests ─────────────────────────────────────────────

fn scope() -> Scope {
    Scope::new(
        nebula_core::WorkspaceId::from_bytes([0x22; 16]).to_string(),
        nebula_core::OrgId::from_bytes([0x11; 16]).to_string(),
    )
}

// ── In-memory store bundle ────────────────────────────────────────────────────

#[derive(Clone)]
struct TestStores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    versions: Arc<InMemoryWorkflowVersionStore>,
    workflows: Arc<InMemoryWorkflowStore>,
}

impl TestStores {
    fn revision_inputs(&self) -> nebula_worker_bin::compose::CoreFlavorRevisionInputs {
        nebula_worker_bin::compose::CoreFlavorRevisionInputs {
            metrics: nebula_metrics::MetricsRegistry::new(),
            artifact_set_digest: nebula_core::ArtifactSetDigest::from_bytes([0x71; 32]),
            catalog: Arc::new(nebula_storage::InMemoryPlanFlavorCatalog::new(
                &self.execution,
            )),
            bundles: Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &self.execution,
            )),
        }
    }

    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        let workflows = InMemoryWorkflowStore::new_with_versions(&versions, &execution);
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            versions: Arc::new(versions),
            workflows: Arc::new(workflows),
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
            operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
                &self.execution,
            )),
        }
    }

    fn workflow_stores(&self) -> nebula_engine::WorkflowStores {
        nebula_engine::WorkflowStores {
            workflow: self.workflows.clone(),
            versions: self.versions.clone(),
        }
    }

    /// Build a handoff over the same shared core used by the queue and execution store.
    ///
    /// The lease write and queue acknowledgement must commit atomically.
    fn turn_handoff(&self) -> Arc<nebula_storage::inmem::InMemoryTurnHandoff> {
        Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(
            &self.execution,
        ))
    }
}

// ── Workflow helpers ──────────────────────────────────────────────────────────

/// Plugin key for the first-party core plugin.
const CORE_PLUGIN_KEY: &str = "core";

async fn activate_set_fields_workflow(stores: &TestStores) -> WorkflowId {
    let workflow_id = WorkflowId::new();
    let now = Utc::now();
    // This fixture submits Start directly; it declares no trigger adapter.
    let def = WorkflowDefinition {
        id: workflow_id,
        name: "smoke-set-fields".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                node_key!("step"),
                "Step",
                CORE_PLUGIN_KEY,
                "core.set_fields",
            )
            .expect("NodeDefinition must build for a valid action key"),
        ],
        connections: Vec::<Connection>::new(),
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::<TriggerBinding>::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores
        .workflows
        .create(
            &scope(),
            nebula_storage_port::dto::WorkflowRecord {
                id: workflow_id.to_string(),
                scope: scope(),
                version: 0,
                slug: "smoke-set-fields".into(),
                deleted: false,
            },
        )
        .await
        .expect("create workflow must succeed");
    let frozen = Arc::new(core_frozen_registry());
    nebula_engine::WorkflowActivationService::new(
        stores.workflows.clone(),
        stores.versions.clone(),
        frozen,
        nebula_engine::PlanFlavorRevisionInstaller::new(Arc::new(
            stores.execution.plan_flavor_catalog(),
        )),
        Arc::new(nebula_core::accessor::SystemClock),
    )
    .activate(&scope(), workflow_id, 0, def)
    .await
    .expect("activation must compile, install and publish the exact core contract");
    workflow_id
}

async fn materialize_start(stores: &TestStores, workflow_id: WorkflowId) -> ExecutionId {
    let starts = Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
        &stores.execution,
    ));
    let service = nebula_engine::WorkflowStartService::new(
        stores.workflow_stores(),
        stores.execution.clone(),
        starts.clone(),
        nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
            stores.execution.plan_flavor_catalog(),
        )),
        Arc::new(core_frozen_registry()),
        Arc::new(nebula_core::accessor::SystemClock),
        nebula_execution::ExecutionBudget::default(),
    )
    .expect("start owner must accept the deployment budget");
    let receipt = service
        .start(
            &scope(),
            workflow_id,
            Some(json!({"fields": [{"name": "greeting", "value": "hello"}]})),
            None,
            None,
        )
        .await
        .expect("start owner must atomically persist the execution and contract");
    let execution_id = receipt.state().execution_id;
    assert_eq!(receipt.state().status, ExecutionStatus::Created);
    let persisted = starts
        .read_contract_bundle(&scope(), &execution_id.to_string())
        .await
        .expect("read persisted bundle")
        .expect("start must retain its exact bundle");
    assert_eq!(
        persisted.record().identity().bundle_id(),
        receipt.bundle().bundle_id()
    );
    let controls = InMemoryControlQueue::new(&stores.execution).snapshot();
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].0.execution_id, execution_id.to_string());
    assert_eq!(
        controls[0].0.command,
        nebula_storage_port::dto::ControlCommand::Start
    );
    execution_id
}

async fn read_status(stores: &TestStores, execution_id: ExecutionId) -> Option<ExecutionStatus> {
    stores
        .execution
        .get(&scope(), &execution_id.to_string())
        .await
        .expect("get execution must succeed")
        .and_then(|r| {
            r.state
                .get("status")
                .and_then(|s| serde_json::from_value::<ExecutionStatus>(s.clone()).ok())
        })
}

fn core_frozen_registry() -> nebula_plugin::FrozenPluginRegistry {
    let plugin = nebula_plugin::ResolvedPlugin::from(
        nebula_plugin_core::CorePlugin::try_new().expect("core manifest"),
    )
    .expect("resolve core");
    let mut registry = nebula_plugin::PluginRegistry::new();
    registry.register(Arc::new(plugin)).expect("register core");
    registry
        .freeze(
            nebula_core::ArtifactSetDigest::from_bytes([0x71; 32]),
            "1.0.0".parse().expect("runtime contract"),
        )
        .expect("freeze core")
}

// ── End-to-end smoke test ─────────────────────────────────────────────────────

/// Proves that the in-memory runtime built by `build_core_flavor_runtime`
/// wires the CorePlugin and processes an owner-materialized Start to `Completed`.
///
/// # Red-ability
///
/// Without the admitted core factory snapshot or persisted bundle, the worker
/// cannot execute the exact plan. Completion and the persisted action output
/// independently prove the owner-created Start reaches the intended action.
#[tokio::test(start_paused = true)]
async fn core_flavor_runtime_processes_materialized_start() {
    let stores = TestStores::new();

    // Build the core-flavor runtime builder. `build_core_flavor_runtime` wires
    // CorePlugin via `engine.with_plugin` and returns a pre-configured
    // `WorkerRuntimeBuilder`, the shared `MetricsRegistry`, and the advertised
    // `PluginKey`.
    let (builder, _metrics, _plugin_key) = build_core_flavor_runtime(
        stores.execution_stores(),
        stores.turn_handoff(),
        stores.turn_handoff(),
        [0xCCu8; 16],
        stores.revision_inputs(),
    )
    .expect("build_core_flavor_runtime must succeed");
    let runtime = builder
        // The same shared core the execution store uses, so the consumer drains
        // the queue this test's API-side writes land in.
        .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
        .build()
        .expect("WorkerRuntimeBuilder::build must succeed with core plugin");

    let workflow_id = activate_set_fields_workflow(&stores).await;
    let execution_id = materialize_start(&stores, workflow_id).await;

    // Spawn the runtime and advance virtual time while the control consumer runs.
    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    // Bounded poll loop: yield + advance virtual time until `Completed` or budget exhausted.
    // 200 iterations × 10 ms virtual = 2 s virtual time. A worker that never ticks fails here.
    let mut completed = false;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        if read_status(&stores, execution_id).await == Some(ExecutionStatus::Completed) {
            completed = true;
            break;
        }
        tokio::time::advance(Duration::from_millis(10)).await;
    }

    cancel.cancel();
    handle
        .await
        .expect("worker task must not panic")
        .expect("every supervised worker component must stop cleanly");

    assert!(
        completed,
        "core-flavor worker did not drive the execution to Completed within the poll budget; \
         ensure build_core_flavor_runtime calls engine.with_plugin(core_plugin)"
    );

    // Assert the action's actual output — proves core.set_fields ran its merge,
    // not just that routing reached a terminal state.
    //
    // The node definition has no parameters, so SetFieldsInput is
    // { data: None, assignments: [] } and the action returns `{}` (empty object).
    // This assertion is the oracle: any silent divergence in action execution
    // (wrong action dispatched, result not persisted) would produce a different
    // value or None here.
    let node_result = stores
        .node_results
        .load_node_result(&scope(), &execution_id.to_string(), "step")
        .await
        .expect("load_node_result must not fail")
        .expect("node result for `step` must be present after Completed");

    // `ActionResult` is tagged with `#[serde(tag = "type")]` and `ActionOutput`
    // with `#[serde(tag = "type", content = "data")]`.
    // `ActionResult::success(v)` → `ActionResult::Success { output: ActionOutput::Value(v) }`
    // serialises as: `{ "type": "success", "output": { "type": "value", "data": <v> } }`.
    // We navigate to the inner data value and assert it equals `{}` — the output of
    // `core.set_fields` with no node parameters (empty assignments list, no base object).
    let output_value = node_result
        .json
        .get("output")
        .and_then(|o| o.get("data"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    assert_eq!(
        output_value,
        json!({}),
        "core.set_fields with no assignments must output an empty object `{{}}`; \
         got `{output_value}` — this proves the action actually executed its merge, \
         not just that the execution reached a terminal state"
    );
}

/// `build_core_flavor_runtime` always produces the `core` plugin key —
/// the flavour binary's contract is that it statically links exactly one plugin.
#[tokio::test]
async fn core_flavor_runtime_advertises_core_plugin_key() {
    let stores = TestStores::new();
    let (_builder, _metrics, key) = build_core_flavor_runtime(
        stores.execution_stores(),
        stores.turn_handoff(),
        stores.turn_handoff(),
        [0x01u8; 16],
        stores.revision_inputs(),
    )
    .expect("build_core_flavor_runtime must succeed with the CorePlugin installed");

    assert_eq!(
        key.as_str(),
        "core",
        "the core-flavor plugin key must be `core`; got `{key}`"
    );
}

/// The evidence-only builder must retain the exact supplied clock and event
/// bus inside the sealed engine while delegating ordinary composition.
#[cfg(feature = "runtime-repair-red")]
#[test]
fn runtime_repair_builder_seals_exact_clock_and_event_bus() {
    let stores = TestStores::new();
    let clock = Arc::new(FixedEvidenceClock {
        wall_time: Utc::now(),
        monotonic: Instant::now(),
    });
    let clock_lifecycle = Arc::downgrade(&clock);
    let clock: Arc<dyn Clock> = clock;
    let event_bus = nebula_eventbus::EventBus::<nebula_engine::ExecutionEvent>::new(8);
    let event_subscriber = event_bus.subscribe();

    let (builder, _, _) = build_core_flavor_runtime_for_runtime_repair_red(
        stores.execution_stores(),
        stores.turn_handoff(),
        stores.turn_handoff(),
        [0xEEu8; 16],
        stores.revision_inputs(),
        nebula_worker_bin::compose::RuntimeRepairEvidenceInputs { clock, event_bus },
    )
    .expect("evidence-specific core flavor builds");
    assert!(
        clock_lifecycle.upgrade().is_some(),
        "sealed engine must retain the exact supplied clock"
    );
    assert!(
        !event_subscriber.is_closed(),
        "sealed engine must retain the supplied event bus"
    );

    let runtime = builder
        .with_control_queue(Arc::new(InMemoryControlQueue::new(&stores.execution)))
        .build()
        .expect("evidence-specific worker builder materializes");
    drop(runtime);
    assert!(
        clock_lifecycle.upgrade().is_none(),
        "clock ownership must end with the sealed runtime"
    );
    assert!(
        event_subscriber.is_closed(),
        "event bus ownership must end with the sealed runtime"
    );
}
