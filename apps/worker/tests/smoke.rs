//! Smoke test: core-flavor boot → with_plugin → claim → drive → complete.
//!
//! Proves the full path:
//!   `compose::build_core_flavor_runtime` (wires CorePlugin + in-memory stores)
//!   → `WorkerRuntime::spawn` → orchestrator claims a `Start` job
//!   → `EngineExecutionSink::dispatch` → `WorkflowEngine::resume_execution`
//!   → execution reaches `Completed`.
//!
//! The test uses in-memory adapters so no SQLite file is created.
//!
//! Red-ability: without `with_plugin` the engine cannot dispatch `core.set_fields`
//! and the execution never reaches `Completed`. The assertion on `completed` will
//! fire, producing a distinct failure message naming the missing plugin wire.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::Utc;
use nebula_core::{WorkflowId, id::ExecutionId, node_key};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_storage::{
    InMemoryExecutionStore, InMemoryWorkflowVersionStore, inmem::InMemoryJobDispatchQueue,
};
use nebula_storage_port::{
    Scope,
    dto::{ControlCommand, JobDispatchMsg},
    store::{ExecutionStore, JobDispatchQueue, NodeResultStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow, Version,
    WorkflowConfig, WorkflowDefinition,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use nebula_worker_bin::compose::build_core_flavor_runtime;

// ── Scope used across these tests ─────────────────────────────────────────────

fn scope() -> Scope {
    Scope::new("nebula", "nebula")
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
}

impl TestStores {
    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
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
            workflow: Arc::new(nebula_storage::InMemoryWorkflowStore::new_with_versions(
                &self.versions,
            )),
            versions: self.versions.clone(),
        }
    }
}

// ── Workflow helpers ──────────────────────────────────────────────────────────

/// Plugin key for the first-party core plugin.
const CORE_PLUGIN_KEY: &str = "core";

async fn save_set_fields_workflow(stores: &TestStores) -> Arc<ValidatedWorkflow> {
    let workflow_id = WorkflowId::new();
    let now = Utc::now();
    // The trigger binding declares a manual trigger intent under the `core` plugin;
    // no trigger action needs to be registered for the engine to execute the workflow
    // node (trigger bindings are metadata for routing, not a registered action dispatch).
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
        trigger_bindings: vec![
            TriggerBinding::new(node_key!("trigger"), CORE_PLUGIN_KEY, "core.trigger.manual")
                .expect("TriggerBinding must build for a valid node key"),
        ],
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    let validated = ValidatedWorkflow::validate(def)
        .expect("set_fields workflow definition must pass validation");
    stores
        .versions
        .create(
            &scope(),
            nebula_storage_port::dto::WorkflowVersionRecord {
                workflow_id: validated.definition().id.to_string(),
                number: 0,
                published: true,
                pinned: false,
                definition: serde_json::to_value(validated.definition())
                    .expect("serialize workflow definition"),
            },
        )
        .await
        .expect("save workflow version must succeed");
    Arc::new(validated)
}

async fn seed_created_execution(
    stores: &TestStores,
    workflow_id: WorkflowId,
    execution_id: ExecutionId,
) {
    let mut state = ExecutionState::new(execution_id, workflow_id, &[]);
    state.set_workflow_input(json!({"fields": [{"name": "greeting", "value": "hello"}]}));
    let state_json = serde_json::to_value(&state).expect("serialize execution state");
    stores
        .execution
        .create(
            &scope(),
            &execution_id.to_string(),
            &workflow_id.to_string(),
            state_json,
        )
        .await
        .expect("create execution row must succeed");
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

// ── End-to-end smoke test ─────────────────────────────────────────────────────

/// End-to-end proof that `build_core_flavor_runtime` wires the CorePlugin into
/// the engine and the claim-loop drives `core.set_fields` to `Completed`.
///
/// # Red-ability
///
/// Without the `with_plugin` call inside `build_core_flavor_runtime`, the engine
/// does not know the `core.set_fields` action key. The orchestrator claims the
/// job and the sink calls `resume_execution`, but the engine returns an
/// `ActionNotFound` error for the unknown key. The execution transitions to
/// `Failed` (or remains `Running` if the engine surfaces an error without a
/// terminal transition), and the `completed` assertion fires with the message
/// "core-flavor worker did not drive the execution to Completed within the poll
/// budget; ensure build_core_flavor_runtime calls engine.with_plugin(core_plugin)".
#[tokio::test(start_paused = true)]
async fn core_flavor_boot_and_drive_to_completed() {
    let stores = TestStores::new();

    // A single shared queue: both the runtime and the seed enqueue share the
    // same in-memory queue so the worker can actually claim the seeded job.
    let queue: Arc<dyn JobDispatchQueue> =
        Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));

    // Build the core-flavor runtime builder. `build_core_flavor_runtime` wires
    // CorePlugin via `engine.with_plugin` and returns a pre-configured
    // `WorkerRuntimeBuilder`, the shared `MetricsRegistry`, and the advertised
    // `PluginKey`. Use a fast poll interval (10 ms) so virtual-time advances fire quickly.
    let (builder, _metrics, plugin_key) = build_core_flavor_runtime(
        stores.execution_stores(),
        stores.workflow_stores(),
        Arc::clone(&queue),
        [0xCCu8; 16],
    )
    .expect("build_core_flavor_runtime must succeed");
    let runtime = builder
        .with_poll_interval(Duration::from_millis(10))
        .build()
        .expect("WorkerRuntimeBuilder::build must succeed with core plugin");

    // Seed a workflow whose sole node is `core.set_fields`.
    let workflow = save_set_fields_workflow(&stores).await;
    let workflow_id = workflow.definition().id;

    // Seed a `Created` execution row.
    let execution_id = ExecutionId::new();
    seed_created_execution(&stores, workflow_id, execution_id).await;

    // Enqueue a `Start` job on the SAME queue the runtime was built with.
    let job_id = [0xAAu8; 16];
    let msg = JobDispatchMsg::new(
        job_id,
        execution_id.to_string(),
        ControlCommand::Start,
        scope(),
        json!({}),
        None::<String>,
        "sha-smoke",
        plugin_key.clone(),
        vec![plugin_key],
        None::<String>,
        0,
    );
    queue
        .enqueue(&msg)
        .await
        .expect("enqueue Start job must succeed");

    // Spawn the runtime with a fast poll interval so virtual-time advances fire quickly.
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
    handle.await.expect("worker task must not panic");

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
    let queue: Arc<dyn JobDispatchQueue> =
        Arc::new(InMemoryJobDispatchQueue::new(&stores.execution));

    let (_builder, _metrics, key) = build_core_flavor_runtime(
        stores.execution_stores(),
        stores.workflow_stores(),
        queue,
        [0x01u8; 16],
    )
    .expect("build_core_flavor_runtime must succeed with the CorePlugin installed");

    assert_eq!(
        key.as_str(),
        "core",
        "the core-flavor plugin key must be `core`; got `{key}`"
    );
}
